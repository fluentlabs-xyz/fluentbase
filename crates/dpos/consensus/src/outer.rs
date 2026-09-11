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
        BeaconEngineLike, DerivedBlockBuilder, ExecutedChain, FluentApp, OrderingAssembler,
    },
    committee::Committee,
    digest::Digest,
    dpos::VoteBackupItem,
    epoch_manager,
    epocher::OriginEpocher,
    executor,
    feed_sink::FeedSink,
    order_block::OrderBlock,
    slasher,
    timeouts::ConsensusTimeouts,
    REPLAY_BUFFER, WRITE_BUFFER,
};
use alloy_primitives::B256;
use commonware_broadcast::buffered;
use commonware_consensus::{
    marshal::{
        self, core::Actor as MarshalActor, resolver::handler as marshal_handler,
        resolver::p2p as marshal_p2p, standard::Standard,
    },
    simplex::types::{Activity, Finalization},
    types::{Epoch, Height, ViewDelta},
    Reporter, Reporters,
};
use commonware_cryptography::{certificate::Provider as CertProvider, ed25519::PublicKey};
use commonware_p2p::{utils::mux::MuxHandle, Blocker, Provider as PeerProvider, Receiver, Sender};
use tracing::{info, warn};

/// A plane-owned `Muxer` broker handle shared across promotions. `MuxHandle::register`
/// takes `&mut self` (a boundary-rate control-channel round-trip), and the move-only
/// `DiscReceiver` breaks the derived `Clone` bound on `MuxHandle`, so the handle is
/// shared via `Arc<Mutex<_>>` (the `Arc` is also the per-promotion `Clone`).
pub(crate) type SharedMux<HS, HR> = Arc<tokio::sync::Mutex<MuxHandle<HS, HR>>>;

/// The validator marshal's backfill/repair resolver. `Plane`: a plain `--dpos`
/// validator (no upstream) catches up on the committee-peer treadmill. `Hybrid`
/// (bug 12): an `--dpos.follower-upstream` validator routes `Finalized{height}`
/// by-height catch-up to the upstream (so an OUT-OF-COMMITTEE joiner with zero
/// consensus-plane connectivity still fills the cold-start gap — the 2026-06-22
/// wedge fix, preserved verbatim) AND live-round `Block(digest)`/`Notarized{round}`
/// repair to the consensus PLANE (the only viable non-finalized body source; a WS
/// upstream holds no notarizations and Fluent's ordering-digest `Block` can't be
/// served from local reth). One concrete `Resolver` type so [`OuterEngine::run`]
/// hands the marshal a single resolver regardless of config. Distinct from
/// [`crate::cert_inlet::FollowerResolver`] (the FOLLOWER path — no plane, no engine,
/// so no `Notarized`/certify seam). Every arm keys on the marshal `Request<Digest>`
/// and ed25519 peers, so the marshal's `verify_delivered` BLS gate is unchanged.
#[derive(Clone)]
enum MarshalResolver<E, U> {
    /// Consensus-plane p2p resolver (no upstream configured).
    Plane(commonware_resolver::p2p::Mailbox<marshal_handler::Request<Digest>, PublicKey>),
    /// A validator-WITH-upstream (bug 12): `Finalized{height}` by-height catch-up
    /// routes to the upstream (preserves the 2026-06-22 out-of-committee fix), while
    /// live-round `Block(digest)` / `Notarized{round}` repair routes to the
    /// consensus PLANE (the only viable non-finalized body source — a follower/WS
    /// upstream holds no notarizations, and Fluent's ordering-digest `Block` can't be
    /// served from local reth). Both deliver into the SAME marshal channel; the
    /// marshal's `verify_delivered` BLS gate is unchanged. See §9.6.
    Hybrid {
        plane: commonware_resolver::p2p::Mailbox<marshal_handler::Request<Digest>, PublicKey>,
        upstream: crate::cert_inlet::UpstreamResolver<E, U>,
    },
}

/// A `Finalized{height}` request routes to the upstream; `Block`/`Notarized`
/// routes to the plane.
fn is_finalized(key: &marshal_handler::Request<Digest>) -> bool {
    matches!(key, marshal_handler::Request::Finalized { .. })
}

impl<E, U> commonware_resolver::Resolver for MarshalResolver<E, U>
where
    E: commonware_runtime::Spawner + commonware_runtime::Metrics + Clone + Send + Sync + 'static,
    U: crate::cert_follow::CertUpstream,
{
    type Key = marshal_handler::Request<Digest>;
    type PublicKey = PublicKey;

    async fn fetch(&mut self, key: Self::Key) {
        match self {
            Self::Plane(r) => r.fetch(key).await,
            Self::Hybrid { plane, upstream } => {
                if is_finalized(&key) {
                    upstream.fetch(key).await
                } else {
                    plane.fetch(key).await
                }
            }
        }
    }
    async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
        match self {
            Self::Plane(r) => r.fetch_all(keys).await,
            Self::Hybrid { plane, upstream } => {
                let (fin, other): (Vec<_>, Vec<_>) = keys.into_iter().partition(is_finalized);
                if !fin.is_empty() {
                    upstream.fetch_all(fin).await;
                }
                if !other.is_empty() {
                    plane.fetch_all(other).await;
                }
            }
        }
    }
    async fn fetch_targeted(
        &mut self,
        key: Self::Key,
        targets: commonware_utils::vec::NonEmptyVec<Self::PublicKey>,
    ) {
        match self {
            Self::Plane(r) => r.fetch_targeted(key, targets).await,
            Self::Hybrid { plane, upstream } => {
                if is_finalized(&key) {
                    upstream.fetch_targeted(key, targets).await
                } else {
                    plane.fetch_targeted(key, targets).await
                }
            }
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
            Self::Plane(r) => r.fetch_all_targeted(requests).await,
            Self::Hybrid { plane, upstream } => {
                let (fin, other): (Vec<_>, Vec<_>) =
                    requests.into_iter().partition(|(k, _)| is_finalized(k));
                if !fin.is_empty() {
                    upstream.fetch_all_targeted(fin).await;
                }
                if !other.is_empty() {
                    plane.fetch_all_targeted(other).await;
                }
            }
        }
    }
    async fn cancel(&mut self, key: Self::Key) {
        match self {
            Self::Plane(r) => r.cancel(key).await,
            // Fan to both — each backend acts only on the subject it owns
            // (`UpstreamResolver::cancel` no-ops on a non-`Finalized` key).
            Self::Hybrid { plane, upstream } => {
                plane.cancel(key.clone()).await;
                upstream.cancel(key).await;
            }
        }
    }
    async fn clear(&mut self) {
        match self {
            Self::Plane(r) => r.clear().await,
            // Dead-defensive (the marshal actor never calls `clear`); fan anyway.
            Self::Hybrid { plane, upstream } => {
                plane.clear().await;
                upstream.clear().await;
            }
        }
    }
    async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
        match self {
            Self::Plane(r) => r.retain(predicate).await,
            // Fan to both. `handler::Request::predicate` KEEPs every cross-subject
            // pair, so each backend prunes only its own subject — no cross-clobber.
            // The two `retain`s run SEQUENTIALLY, so the shared predicate is behind a
            // `Mutex` (the bound is `Send + 'static`, NOT `Clone`/`Sync`): no contention.
            Self::Hybrid { plane, upstream } => {
                let pred = Arc::new(Mutex::new(predicate));
                let p = pred.clone();
                plane.retain(move |k| (p.lock().unwrap())(k)).await;
                upstream.retain(move |k| (pred.lock().unwrap())(k)).await;
            }
        }
    }
}

use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::paged::CacheRef, spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics,
    Network as RNetwork, Spawner, Storage,
};
use commonware_storage::archive::{immutable, Archive as _, Identifier};
use commonware_utils::{NZUsize, NZU16, NZU64};
use fluentbase_bls::{keys::ValidatorBlsKeypair, PeerPubkey, Scheme as BlsScheme};
use rand_core::CryptoRngCore;
use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

pub(crate) const PAGE_CACHE_PAGE_SIZE: std::num::NonZeroU16 = NZU16!(4_096);
pub(crate) const PAGE_CACHE_CAPACITY: NonZeroUsize = NZUsize!(8_192);
const IMMUTABLE_ITEMS_PER_SECTION: NonZeroU64 = NZU64!(262_144);
const PRUNABLE_ITEMS_PER_SECTION: NonZeroU64 = NZU64!(4_096);
pub(crate) const MAX_REPAIR: NonZeroUsize = NZUsize!(20);
/// Marshal dispatch-ahead window (`PendingAcks::has_capacity`): at most this many
/// finalized bodies are dispatched to the executor ahead of the acks flowing back.
/// In steady state the executor derives and acks each body at its own delivery,
/// so no slot is held; a block whose σ has not landed yet holds exactly ONE
/// (the executor's `awaiting_seed` slot), leaving 15 of slack. `has_capacity`
/// reserves only the OLDEST un-acked block, so dispatch never stalls on it.
pub(crate) const MAX_PENDING_ACKS: NonZeroUsize = NZUsize!(16);
const FREEZER_TABLE_RESIZE_FREQUENCY: u8 = 4;
const FREEZER_TABLE_RESIZE_CHUNK_SIZE: u32 = 1 << 16;
const FREEZER_VALUE_TARGET_SIZE: u64 = 1 << 30;
const FREEZER_VALUE_COMPRESSION: Option<u8> = Some(3);

/// A VIEW on the committee module's single map — the marshal's
/// [`CertProvider`], the executor's finalization-refetch peer source and the
/// repair sweep's work list, all answered from the one place an epoch's scheme
/// lives.
///
/// It used to BE the registry: its own `BTreeMap<Epoch, Arc<BlsScheme>>`, its
/// own count-based retention, and four producers writing into it from four
/// different cursors. Every one of those is gone. The map is
/// [`crate::committee::Committee`]'s, the retention is the read window, and the
/// producers are two — the module's own verifier at install, and
/// [`Committee::upgrade_scheme`] for the signer half. What is left here is a
/// name and three projections, kept as a type so the marshal's `CertProvider`
/// bound and the metric names it is known by do not have to move too.
#[derive(Clone)]
pub struct EpochSchemeProvider {
    committee: Arc<dyn Committee>,
}

impl EpochSchemeProvider {
    pub fn new(committee: Arc<dyn Committee>) -> Self {
        Self { committee }
    }

    /// Registered epochs whose scheme is VERIFY-ONLY — the repair sweep's
    /// candidate list. See [`Committee::verifier_epochs`].
    pub fn verifier_epochs(&self) -> Vec<Epoch> {
        self.committee
            .verifier_epochs()
            .into_iter()
            .map(Epoch::new)
            .collect()
    }

    /// The scheme for the highest known epoch (the current committee). Its
    /// `participants()` are the peers to target for a finalization re-fetch on
    /// catch-up — they are connected and hold the durable finalizations.
    pub fn latest_scheme(&self) -> Option<Arc<BlsScheme>> {
        self.committee.latest_scheme()
    }
}

impl CertProvider for EpochSchemeProvider {
    type Scope = Epoch;
    type Scheme = BlsScheme;

    fn scoped(&self, scope: Epoch) -> Option<Arc<BlsScheme>> {
        self.committee.scheme(scope.get())
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
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork,
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
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork,
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
pub struct OuterBuilder<B, P, BE, D, XC, A> {
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
    /// The ONE randomness handle, built by `beacon::build` and handed in whole.
    /// It replaced six separate beacon fields — the resolver, the share edge, the
    /// key map, the two agreement rungs and the verify context — none of which
    /// this layer can name any more.
    pub randomness: std::sync::Arc<dyn crate::beacon::Beacon>,
    /// Edge-trigger the executor fires when it records a finalized block — the
    /// mid-epoch promotion trigger. Threaded to BOTH the executor (producer) and
    /// the manager (consumer).
    pub spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// Steady-state self-healing re-jump callback (see [`executor::ReJump`]),
    /// threaded into the executor's `Config`. `Some` on any upstream-configured
    /// node (follower or validator-with-upstream); `None` for a plain validator
    /// and in tests.
    pub re_jump: Option<executor::ReJump>,
    /// Membership / `Inline::genesis` counters (cross-launch singleton from
    /// `dpos.rs::launch`, already registered there). Core-owned on BOTH node
    /// classes.
    pub epoch_metrics: crate::epoch_manager::EpochEngineMetrics,
    /// The executor's per-derived-block seed observation (cross-launch singleton
    /// from `dpos.rs::launch`, already registered there). Executor-owned on BOTH
    /// node classes — the executor runs on both and cannot tell which.
    pub executor_metrics: crate::executor::ExecutorMetrics,
    /// Self-heal stuck-detector (cross-launch singleton from `dpos.rs::launch`,
    /// already registered there). Threaded to the executor for the #14 finalize-FCU
    /// transport-retry + #1 steady-state re-jump `AuthFailed` rotate gauges.
    pub sync_metrics: crate::sync_metrics::SyncMetrics,
    /// Fork-safety latch (Phase 3 `SafetyHalt`, cross-launch singleton from
    /// `dpos.rs::launch`). Threaded to the executor (engages it on divergence /
    /// EL-Invalid / L1-fork), to `epoch_manager` (never re-promote a halted node),
    /// and kept on the [`OuterEngine`] so the supervisor PARKS (keeps marshal +
    /// `consensus`-RPC alive) on a halt instead of abort-all.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// Committee members observed slashed for equivocation — filled by the node's
    /// tombstone watcher off chain state, read by `FluentApp` to refuse binding
    /// their proposals and to drop a charge whose verdict already landed. Passed
    /// in rather than defaulted here because the writer lives in the node crate;
    /// a default would be a set nothing fills.
    pub tombstones: crate::slasher::TombstoneSet,
    /// The ordering-vs-DKG clock pair. Threaded to `FluentApp`, which writes the
    /// ordering half off marshal's tip — the ONE observer of finalization that
    /// survives a `SafetyHalt` park and every engine abort.
    pub plane_clock: crate::sync_metrics::PlaneClock,
    /// The beacon plane's height channel. `FluentApp` feeds marshal's ordering tip
    /// into it — the plane's third clock feeder, and the one that survives an
    /// execution stall. `None` where no beacon plane runs (a follower, a test).
    pub dkg_height_tx: Option<tokio::sync::mpsc::Sender<u64>>,
    pub timeouts: ConsensusTimeouts,
    pub mailbox_size: usize,
    pub deque_size: usize,
    /// Prefix of the marshal's archive partitions (`{prefix}-v3-…`,
    /// `{prefix}-v2-…`; `consensus_marshal` in production).
    pub partition_prefix: String,
    /// Prefix of the PER-EPOCH journal partitions the epoch manager opens:
    /// `{prefix}consensus_epoch_{E}` and `{prefix}dkg_epoch_{E}` — see
    /// [`epoch_manager::Config::partition_prefix`]. Production passes `""`
    /// (on-disk names unchanged); the in-crate deterministic testbed passes
    /// `node{i}-`. Separate from [`Self::partition_prefix`] because the marshal
    /// prefix is a NAME (`consensus_marshal`) and this one is a namespace
    /// prepended to names.
    pub engine_partition_prefix: String,
    pub resolver_initial: Duration,
    pub resolver_timeout: Duration,
    pub resolver_fetch_retry: Duration,

    // FluentApp constructor args.
    pub genesis: OrderBlock,
    pub beacon_engine: BE,
    /// OrderBlock → derived-EVM-block execution (node-side, reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-side, provider-backed, by-NUMBER).
    pub executed: XC,
    /// Pool-backed ordering assembly with the in-flight suffix overlay.
    pub assembler: Arc<A>,
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
    /// When migrating (sequencer→DPoS), the anchor height to seed the marshal's
    /// in-order dispatch floor so it does not backfill pre-anchor history it
    /// will never receive. `None` on normal restart (floor comes from metadata).
    pub marshal_floor: Option<Height>,
    /// Authenticated by-height seam used to seed the epoch-boundary block(s) that
    /// [`Self::marshal_floor`] is about to bury. `None` when no upstream is
    /// configured.
    pub boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn>,
    /// The `PK_epoch` ladder's artifact rungs, handed to the epoch manager:
    /// Epoch-entry seam — the height-keyed half of [`Self::boundary_hook`], handed to the
    /// executor so a steady-state re-jump enters its LANDING epoch. Without it the landing
    /// epoch is entered only at the NEXT boundary, leaving a seated member verify-only for
    /// the remainder of the epoch it landed in. Tests pass `Arc::new(|_| {})`.
    pub boundary_enter: Arc<dyn Fn(u64) + Send + Sync>,
    /// Read-floor seam — the height-floor twin of [`Self::boundary_enter`] on the same
    /// `EpochTransition`. A re-jump landing publishes `landing − K`, and the state machine
    /// clamps every later committee read to it; without that the entry above reads state
    /// the jump left behind, which a pruned node no longer has. Awaited by the executor
    /// immediately before the entry. Tests pass `Arc::new(|_| Box::pin(async {}))`.
    pub boundary_read_floor: crate::executor::BoundaryReadFloorFn,
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
    /// Every per-epoch committee read this engine makes: the slasher's evidence
    /// resolve, and the executor's "the anchor moved" wake-up. ONE handle, and
    /// the same one the beacon plane's `CommitteeReads` facade views — the
    /// slasher used to carry its own reader plus its own finalized-hash closure,
    /// which is a second cursor over the same contract array.
    pub committee: Arc<dyn crate::committee::Committee>,
    /// TxPool transport (signer + pool + provider wrapper from dpos.rs).
    pub slasher_sink: std::sync::Arc<dyn slasher::actor::SlasherTxSink>,
    /// WAL storage partition name. The actual `queue::shared` handles
    /// are initialised inside [`OuterBuilder::build`] under the slasher's
    /// own context label.
    pub slasher_wal_partition: String,
    /// Evidence-channel bridge to the node's gossip task
    /// ([`slasher::gossip`]). `None` on the follower path, whose slasher is
    /// constructed but never started.
    pub slasher_evidence: Option<slasher::EvidenceBridge>,
    /// Supervisor handles of the epoch-key agreement instances the beacon plane
    /// starts, so [`epoch_manager::Actor`] prunes them on the SAME frontier cutoff
    /// as the per-epoch engines and tears them down with itself. `None` ⇒ no
    /// agreement plane wired (the follower path, and any test) ⇒ the manager's
    /// intake branch parks forever and its map stays empty.
    ///
    /// A passthrough and nothing else: the receiver is move-only and the manager
    /// is constructed inside [`OuterBuilder::build`], so there is no other way for
    /// the plane to hand it over. The instances themselves never enter the
    /// supervisor's `select!` — they are SUPPOSED to complete, and a completed
    /// handle there would read as a dead subsystem.
    pub agreement_intake: Option<mpsc::Receiver<(Epoch, Handle<()>)>>,

    /// DEVNET/TEST-ONLY byzantine validator behaviour (gated behind
    /// `dpos-devnet-byzantine`). `None` on every honest node. Threaded into
    /// [`epoch_manager::Config`] so the per-epoch engine can swap in a
    /// [`crate::byzantine::VoteEquivocator`].
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
}

/// The global-singleton consensus driver wrapping a per-epoch
/// [`epoch_manager::Actor`].
pub struct OuterEngine<E, B, P, BE, D, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork,
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    A: OrderingAssembler,
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
    slasher: slasher::Actor<E>,
    boundary_tx: mpsc::Sender<Epoch>,
    me: PublicKey,
    blocker: B,
    provider: P,
    mailbox_size: usize,
    resolver_initial: Duration,
    resolver_timeout: Duration,
    resolver_fetch_retry: Duration,
    /// Fork-safety latch — the supervisor checks it on a subsystem exit: engaged ⇒
    /// a Phase-3 SafetyHalt (park, keep marshal/RPC alive); not engaged ⇒ a real
    /// crash (abort-all).
    safety_halt: crate::sync_metrics::SafetyHalt,
    /// Held for one reason: the by-height resolver captures the σ of every
    /// certificate it pulls, and that resolver is built here rather than in
    /// `build`.
    randomness: std::sync::Arc<dyn crate::beacon::Beacon>,
}

/// What the OuterEngine supervisor does when the FIRST subsystem handle resolves.
#[derive(Debug, PartialEq, Eq)]
enum SupervisorAction {
    /// A genuine subsystem exit/crash (the latch is NOT engaged): abort the other
    /// subsystems to release runtime resources; `run` returns and its caller tears
    /// the process down. Fork-safety is unaffected — this is the normal crash path.
    AbortAll,
    /// A Phase-3 SafetyHalt is engaged: the node must STAY UP + observable — keep
    /// marshal + `consensus`-RPC + `epoch_manager` alive; do NOT abort-all. The
    /// supervisor parks (awaits the surviving handles) until a real external
    /// shutdown. BACKSTOP only since the executor ack-retention park: a halted
    /// executor no longer exits (it parks in-task retaining marshal acks —
    /// `executor::Actor::park_halted`), so this arm fires only if some OTHER
    /// subsystem exits while the latch is engaged.
    ParkHalted,
}

/// Never-returning supervisor park for the SafetyHalt backstop.
///
/// Takes the surviving handles BY VALUE and parks: commonware's `Handle` has no
/// `Drop` impl, so holding them keeps their tasks alive, while *awaiting* them
/// would re-poll the handle that already resolved in the supervisor's `select!`
/// — `Handle::poll` forwards to a tokio oneshot that panics "called after
/// complete", turning "stay up, verify-only" into a task panic and, under
/// `with_catch_panics(true)`, a full node teardown.
async fn park_supervisor(_surviving: Vec<Handle<()>>) {
    std::future::pending::<()>().await
}

/// The supervisor decision, factored out so the fork-safety property — a
/// SafetyHalt parks (stays up) while a real crash aborts-all — is unit-testable
/// without standing up the whole generic engine.
fn supervisor_action(safety_halt: &crate::sync_metrics::SafetyHalt) -> SupervisorAction {
    if safety_halt.is_engaged() {
        SupervisorAction::ParkHalted
    } else {
        SupervisorAction::AbortAll
    }
}

impl<B, P, BE, D, XC, A> OuterBuilder<B, P, BE, D, XC, A>
where
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    /// Construct the engine in dependency order:
    /// `buffered + archives + scheme_provider → marshal → executor →
    /// FluentApp → epoch_manager`.
    ///
    pub async fn build<E>(self, context: E) -> eyre::Result<OuterEngine<E, B, P, BE, D, XC, A>>
    where
        E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork,
    {
        // Fail loud and early on misconfigured timeouts so a deep panic
        // inside commonware (`voter/actor.rs:136`) becomes an actionable
        // startup error instead.
        self.timeouts
            .validated()
            .expect("ConsensusTimeouts invariants violated");
        // The production record encodes `leader_index` as a u8, and the staking
        // module's `recordProduction(uint8)` mirrors that width. A committee
        // larger than 255 would make an honest leader's index unencodable, so
        // every voter would reject every block it proposed — a silent,
        // permanent loss of that member's whole slot share. Fail at startup
        // instead, before any block is proposed. The cap is ONE declaration,
        // `fluentbase_types::staking_protocol::MAX_COMMITTEE_SIZE`, which the
        // staking module imports under the same name — so this asserts a
        // configured value, not an agreement between two literals.
        assert!(
            fluentbase_p2p::constants::MAX_COMMITTEE_SIZE <= u8::MAX as u64,
            "wire format requires leader_index to fit u8; \
             MAX_COMMITTEE_SIZE = {} exceeds 255 — widen the production record \
             before bumping",
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

        let finalized_blocks =
            init_finalized_blocks_archive(&context, &self.partition_prefix).await;

        // Single cross-epoch FixedEpocher + scheme provider. The same
        // instance is threaded into marshal::Config below AND into
        // epoch_manager::Config so all per-epoch engines + marshal share
        // one source of truth — no risk of divergent epoch math after a
        // hypothetical interval re-read (defense-in-depth).
        // The randomness handle arrives BUILT — this layer no longer assembles it
        // from six separate beacon handles, because it no longer holds them: the
        // key store, the seed store, both resolvers and the two agreement rungs
        // are created inside `beacon::build` and never cross back out.
        let randomness = self.randomness;
        let scheme_provider = EpochSchemeProvider::new(self.committee.clone());
        let epocher = OriginEpocher::new(self.dpos_activation_block, self.epoch_length_blocks);

        // Which epoch-boundary heights the floor about to be applied would bury, and
        // which of those we do not already hold. MUST run here: `MarshalActor::init`
        // below takes both archives by value, and the read has to happen before the
        // floor is raised because the WRITE window closes at `SetFloor`
        // (`store_finalization` drops `height <= last_processed_height`) even though
        // the READ side never consults the floor.
        //
        // Keyed on the CONDITION ("a terminal at/below the floor is missing"), not on
        // the event "a jump just landed": a second restart inside the same landing
        // epoch takes the small-gap `Lagging` arm and carries no jump outcome, yet the
        // hole — and the verify-only parking it causes — is identical.
        let boundary_seed_heights: Vec<u64> = match (self.marshal_floor, &self.boundary_fetch) {
            (Some(floor), Some(_)) => {
                let mut wanted = Vec::new();
                if let Some(b) = epocher.terminal_at_or_below(floor) {
                    // `b` feeds `Inline::genesis(E)` and the engine-spawn gate,
                    // and it is the ONE height this seeding needs. It used to
                    // fetch `b + 1` alongside it, because the promote VALUE-gate
                    // read the epoch's first block for the network-attested key;
                    // that gate now compares against the agreement artifact,
                    // which no block carries and no floor can bury.
                    let h = b.get();
                    if h <= floor.get() {
                        let present = finalized_blocks
                            .get(Identifier::Index(h))
                            .await
                            .unwrap_or(None)
                            .is_some();
                        if !present {
                            wanted.push(h);
                        }
                    }
                }
                wanted
            }
            _ => Vec::new(),
        };

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

        // sequencer→DPoS swap: seed the marshal's in-order floor to the anchor so it
        // dispatches from anchor+1 instead of chasing pre-anchor history that no
        // DPoS node holds (would otherwise stall Update::Block forever). Buffered
        // in the mailbox until the marshal actor starts in `run`; SetFloor is
        // raises-only, so this is a no-op on a normal restart that passed the flag.
        // Seed the boundary block(s) the floor below is about to bury — BEFORE it is
        // raised. The ordering IS the mechanism: the two store messages and `SetFloor`
        // share one mpsc that the actor drains a message per loop turn, and the write
        // gate is evaluated at message-processing time, so a store enqueued ahead of
        // `SetFloor` lands. It then stays readable forever — immutable-archive `prune`
        // is a no-op and the by-height read consults no floor — which is what lets the
        // engine-spawn gate and `Inline::genesis` find it afterwards.
        //
        if !boundary_seed_heights.is_empty() {
            let fetch = self
                .boundary_fetch
                .clone()
                .expect("boundary_seed_heights is only non-empty when the seam exists");
            let at_hash = self.initial_finalized.1;
            let mut fetched = Vec::with_capacity(boundary_seed_heights.len());
            for height in &boundary_seed_heights {
                let Some(uf) = fetch(*height, at_hash).await else {
                    fetched.clear();
                    break;
                };
                fetched.push(uf);
            }
            if fetched.is_empty() {
                warn!(
                    heights = ?boundary_seed_heights,
                    "epoch-boundary seeding incomplete — injecting nothing; this member stays \
                     verify-only (no proposals, no votes) until the next epoch boundary"
                );
            }
            let mut inject = marshal_mailbox.clone();
            for uf in fetched {
                let height = uf.block.height;
                let round = uf.finalization.proposal.round;
                inject.verified(round, uf.block).await;
                Reporter::report(&mut inject, Activity::Finalization(uf.finalization)).await;
                self.sync_metrics.jump_boundary_refetched.inc();
                info!(
                    height,
                    "seeded epoch-boundary block below the marshal floor so this member can \
                     spawn its engine in the landing epoch"
                );
            }
        }

        if let Some(floor) = self.marshal_floor {
            marshal_mailbox.set_floor(floor).await;
        }

        // Reject crash-recovery backfill ranges that cross an epoch
        // boundary — per-epoch BLS scheme rotation across backfill is
        // not yet supported.
        // Loud operator-actionable error beats a silent wrong-snapshot
        // read from staking_reader. Empty range = no-op for the
        // sequencer→DPoS migration path (cons_fin = 0).
        let backfill_range =
            (self.last_execution_finalized_height + 1)..=last_consensus_finalized_height.get();
        if !backfill_range.is_empty() {
            // `epoch_length_blocks` is a `NonZeroU64` and the shared epoch
            // function takes the interval at that width, so the narrowing this
            // used to guard against is gone. A `NonZeroU64` can never make the
            // function answer `None`.
            let epoch_interval = self.epoch_length_blocks.get();
            let backfill_start_epoch = fluentbase_staking_reader::reader::epoch_at_block(
                self.last_execution_finalized_height,
                self.dpos_activation_block,
                epoch_interval,
            );
            let backfill_end_epoch = fluentbase_staking_reader::reader::epoch_at_block(
                last_consensus_finalized_height.get(),
                self.dpos_activation_block,
                epoch_interval,
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
                // The SAME value the buffered `SetFloor` above carries, so the
                // stale-dispatch guard is live from tick zero rather than from the
                // first `reseed_forward` — `MarshalActor::run` dispatches once BEFORE
                // it processes that `SetFloor`.
                initial_marshal_floor: self.marshal_floor.map_or(0, |h| h.get()),
                boundary_fetch: self.boundary_fetch.clone(),
                boundary_enter: self.boundary_enter,
                boundary_read_floor: self.boundary_read_floor,
                dpos_activation_block: self.dpos_activation_block,
                fcu_pace: self.fcu_pace,
                peers_for_finalization: peers_for_finalization.clone(),
                metrics: self.executor_metrics.clone(),
                sync_metrics: self.sync_metrics.clone(),
                safety_halt: self.safety_halt.clone(),
                spawn_unblocked: self.spawn_unblocked.clone(),
                re_jump: self.re_jump,
                randomness: randomness.clone(),
                epocher: epocher.clone(),
                // The committee module's anchor wake-up. Erased to one verb here
                // rather than handing the executor the module itself, so the
                // executor gains no committee read of its own.
                anchor_advanced: {
                    let committee = self.committee.clone();
                    Arc::new(move || committee.anchor_advanced())
                },
            },
        );

        // Notarization arm of the simplex reporter — forwards `SpecNotarized`
        // to the executor for speculative execution. Built from a mailbox clone
        // before `FluentApp` consumes `executor_mailbox`. Also writes the recovered
        // seed into `seed_store` for the certify gate.
        let spec_exec_mailbox =
            crate::spec_exec::Mailbox::new(executor_mailbox.clone(), randomness.clone());

        // The slasher's verified-charge queue: filled by the slasher below,
        // drained one charge per block by the proposer. Created here because the
        // app is built before the slasher and both need the SAME handle — a
        // second store would be a queue nothing fills.
        let charges = slasher::ChargeStore::default();

        // FluentApp (needs executor_mailbox + marshal_mailbox + sidecar state).
        // The beacon seed feed lives here, NOT on the executor: the partial is
        // triggered at notarize-time (verify→true / own propose), so seed(h) is
        // recovered by the time h finalizes (sign-at-notarize).
        let app = FluentApp::new(
            self.genesis,
            executor_mailbox,
            self.boundary_hook,
            self.executed,
            self.assembler,
            self.target_gas_limit,
            self.dpos_activation_block,
            self.chain_id,
            Some(charges.clone()),
            self.tombstones,
        )
        .with_plane_clock(self.plane_clock);
        let app = match self.dkg_height_tx {
            Some(tx) => app.with_dkg_heights(tx),
            None => app,
        };
        let marshal_reporter_app = app.clone();

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
                committee: self.committee.clone(),
                // TxPool transport (signer + pool + provider).
                sink: self.slasher_sink,
                // Durable WAL split between producer/consumer tasks.
                wal_writer,
                wal_reader,
                // `Actor::init` binds its own mailbox into the bridge, closing
                // the inbound direction of the evidence channel.
                evidence: self.slasher_evidence,
                // The same handle the app reads at propose time.
                charges,
            },
        );

        // EpochManager — gets the SAME FixedEpocher instance threaded
        // through to its per-epoch engines (single source of truth).
        let (epoch_manager, boundary_tx) = epoch_manager::Actor::new(
            context.with_label("epoch_manager"),
            epoch_manager::Config {
                randomness: randomness.clone(),
                me: self.me.clone(),
                blocker: self.blocker.clone(),
                chain_id: self.chain_id,
                epocher: epocher.clone(),
                signer_keypair: self.signer_keypair,
                app,
                timeouts: self.timeouts,
                mailbox_size: self.mailbox_size,
                spawn_unblocked: self.spawn_unblocked,
                safety_halt: self.safety_halt.clone(),
                marshal_mailbox: marshal_mailbox.clone(),
                peers_for_finalization,
                slasher_mailbox,
                spec_exec_mailbox,
                epoch_metrics: self.epoch_metrics,
                page_cache,
                committee: self.committee.clone(),
                scheme_pins: scheme_provider.clone(),
                partition_prefix: self.engine_partition_prefix,
                #[cfg(feature = "dpos-devnet-byzantine")]
                byzantine: self.byzantine,
            },
        );

        let epoch_manager = match self.agreement_intake {
            Some(intake) => epoch_manager.with_agreement_intake(intake),
            None => epoch_manager,
        };

        Ok(OuterEngine {
            randomness: randomness.clone(),
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
            me: self.me,
            blocker: self.blocker,
            provider: self.provider,
            mailbox_size: self.mailbox_size,
            resolver_initial: self.resolver_initial,
            resolver_timeout: self.resolver_timeout,
            resolver_fetch_retry: self.resolver_fetch_retry,
            safety_halt: self.safety_halt,
        })
    }
}

impl<E, B, P, BE, D, XC, A> OuterEngine<E, B, P, BE, D, XC, A>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork,
    B: Blocker<PublicKey = PublicKey> + Clone,
    P: PeerProvider<PublicKey = PublicKey> + Clone,
    BE: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    /// Sender held by 03's `EpochTransition` to fire boundary triggers.
    ///
    /// The EPOCH and nothing else: the committee that epoch's reconcile needs is
    /// read from the committee module, at this node's own anchor, so a snapshot
    /// riding this channel would be a second copy of a value the module already
    /// froze — and a second copy is a value two nodes can disagree on.
    pub fn boundary_sender(&self) -> mpsc::Sender<Epoch> {
        self.boundary_tx.clone()
    }

    /// Clone of the marshal mailbox for the node-side cert feed/RPC. Call before
    /// [`OuterEngine::start`] (which consumes `self`) and hand it up to the node
    /// so its feed actor can answer `get_finalization`+`get_block` by height.
    pub fn marshal_mailbox(&self) -> MarshalMailbox {
        self.cert_mailbox.clone()
    }

    /// Broker-handle start. Threads the 5 plane-owned `MuxHandle`s + this
    /// promotion's vote-backup receiver:
    ///   vote/cert/resolver → EpochManager (per-epoch register/deregister)
    ///   broadcast → buffered::Engine (subchannel 0, registered once in `run`)
    ///   marshal → marshal::resolver::p2p::init → marshal::core::Actor (subchannel 0)
    /// Registration is async, so it happens inside `run` (keeps `start` sync); the
    /// muxes live in the always-on plane, so on demote the dropped `SubReceiver`s
    /// auto-deregister and the next promotion re-registers — restart-free.
    ///
    /// `upstream`: `Some` for an upstream-configured validator
    /// (`--dpos.follower-upstream`) — the marshal's by-height backfill resolver is
    /// the UPSTREAM-backed [`UpstreamResolver`](crate::cert_inlet::UpstreamResolver)
    /// instead of the consensus-plane p2p resolver, so an OUT-OF-COMMITTEE node
    /// (no consensus-plane connectivity — the peer set == committee) can still fill
    /// the cold-start `[floor+1 .. first_live]` gap (the validator-with-upstream
    /// wedge fix; the upstream serves ALL finalized certs by-height, in BOTH in- and
    /// out-of-committee states). `None` for a plain `--dpos` validator — it keeps the
    /// p2p resolver (it catches up on the consensus-plane treadmill). In either case
    /// the marshal BLS-verifies every delivered cert against the per-epoch committee
    /// (`verify_delivered`), so trustlessness is intact, and the resolver only
    /// DELIVERS into the marshal — the executor stays the sole reth writer.
    #[allow(clippy::too_many_arguments)]
    pub fn start<HS, HR, U>(
        mut self,
        ctx_for_resolver: E,
        vote_mux: SharedMux<HS, HR>,
        cert_mux: SharedMux<HS, HR>,
        resolver_mux: SharedMux<HS, HR>,
        broadcast_mux: SharedMux<HS, HR>,
        marshal_mux: SharedMux<HS, HR>,
        vote_backup: mpsc::Receiver<VoteBackupItem>,
        upstream: Option<U>,
    ) -> Handle<()>
    where
        E: Clone + Sync,
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
        U: crate::cert_follow::CertUpstream,
    {
        spawn_cell!(
            self.context,
            self.run(
                ctx_for_resolver,
                vote_mux,
                cert_mux,
                resolver_mux,
                broadcast_mux,
                marshal_mux,
                vote_backup,
                upstream,
            )
            .await
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn run<HS, HR, U>(
        self,
        ctx_for_resolver: E,
        vote_mux: SharedMux<HS, HR>,
        cert_mux: SharedMux<HS, HR>,
        resolver_mux: SharedMux<HS, HR>,
        broadcast_mux: SharedMux<HS, HR>,
        marshal_mux: SharedMux<HS, HR>,
        vote_backup: mpsc::Receiver<VoteBackupItem>,
        upstream: Option<U>,
    ) where
        E: Clone + Sync,
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
        U: crate::cert_follow::CertUpstream,
    {
        // Register subchannel 0 on the broadcast mux (the global singleton that
        // consumes one fixed sub-channel). On a `Closed` error (plane Muxer torn
        // down at shutdown) the engine exits — the supervisor is already shutting
        // down.
        let broadcast = match broadcast_mux.lock().await.register(0).await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!(error = ?e, "broadcast mux register(0) failed — OuterEngine exiting");
                return;
            }
        };

        // The marshal's by-height backfill resolver. Two shapes:
        //   * UPSTREAM-backed ([`UpstreamResolver`]) when `--dpos.follower-upstream`
        //     is configured — an OUT-OF-COMMITTEE validator (a not-yet-committee
        //     external joiner) has ZERO consensus-plane connectivity (the tracked
        //     peer set == the on-chain committee), so the p2p resolver below could
        //     never fetch the cold-start `[floor+1 .. first_live]` gap and the
        //     executor would wedge (the same class as the follower wedge). The
        //     upstream serves ALL finalized certs by-height — in BOTH in- and
        //     out-of-committee states — so it is the marshal backfill source for the
        //     WHOLE process lifetime, not just while out-of-committee (an
        //     in-committee signer's local engine produces certs live; the resolver
        //     only ever fires for catch-up gaps the upstream can equally serve). The
        //     resolver (inside `marshal_chan`, owned by the marshal for its lifetime)
        //     holds its own `upstream` clone, which keeps the WS actor alive (it exits
        //     when all handles drop).
        //   * Consensus-plane p2p ([`marshal_p2p::init`]) for a plain `--dpos`
        //     validator — it catches up on the committee-peer treadmill (no upstream
        //     to pull from). This registers subchannel 0 on the marshal mux.
        // Either way the marshal BLS-verifies every delivered cert against the
        // per-epoch committee (`verify_delivered`) — trustless — and the resolver
        // only DELIVERS into the marshal; the executor stays the sole reth writer.
        let marshal_chan = match upstream {
            Some(up) => {
                // HYBRID (bug 12): a validator-with-upstream keeps the consensus-plane
                // resolver for live-round `Block`/`Notarized` repair AND the by-height
                // `UpstreamResolver` for `Finalized` catch-up. Both DELIVER into the
                // SAME marshal channel; the plane owns its own delivery channel, so a
                // tiny forwarder folds its deliveries into the shared one.
                let (marshal_tx, marshal_rx) =
                    mpsc::channel::<marshal_handler::Message<Digest>>(self.mailbox_size.max(1));
                let up_handler = marshal_handler::Handler::<Digest>::new(marshal_tx.clone());
                let upstream = crate::cert_inlet::UpstreamResolver::new(
                    ctx_for_resolver.clone(),
                    up,
                    up_handler,
                    self.randomness.clone(),
                );
                let marshal_sub = match marshal_mux.lock().await.register(0).await {
                    Ok(sub) => sub,
                    Err(e) => {
                        tracing::error!(error = ?e, "marshal mux register(0) failed (hybrid) — OuterEngine exiting");
                        return;
                    }
                };
                let (plane_rx, plane) = marshal_p2p::init::<_, _, _, Digest, _, _, _>(
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
                    marshal_sub,
                );
                ctx_for_resolver
                    .with_label("hybrid_plane_deliver")
                    .spawn(move |_| async move {
                        let mut plane_rx = plane_rx;
                        while let Some(msg) = plane_rx.recv().await {
                            if marshal_tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                    });
                (marshal_rx, MarshalResolver::Hybrid { plane, upstream })
            }
            None => {
                let marshal_sub = match marshal_mux.lock().await.register(0).await {
                    Ok(sub) => sub,
                    Err(e) => {
                        tracing::error!(error = ?e, "marshal mux register(0) failed — OuterEngine exiting");
                        return;
                    }
                };
                let (marshal_rx, marshal_resolver) = marshal_p2p::init::<_, _, _, Digest, _, _, _>(
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
                    marshal_sub,
                );
                (marshal_rx, MarshalResolver::Plane(marshal_resolver))
            }
        };

        // Start `epoch_manager` FIRST so its `boundary_rx` is
        // draining before `marshal` starts firing the `Update::Block`
        // path that ultimately triggers `boundary_hook` → bridge_tx.
        // The bridge channel buffers 64 triggers (`dpos.rs` `bridge_tx`/`bridge_rx`) which
        // absorbed the original ordering gap, but starting epoch_manager
        // first eliminates the window for live epoch transitions when
        // bursty finalization races a still-uninitialized consumer.
        let mut em_handle = self.epoch_manager.start(
            Some(epoch_manager::Muxes {
                vote: vote_mux,
                cert: cert_mux,
                res: resolver_mux,
            }),
            vote_backup,
        );
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

        match supervisor_action(&self.safety_halt) {
            SupervisorAction::ParkHalted => {
                // Phase-3 SafetyHalt BACKSTOP: a halted executor parks in-task
                // (retaining marshal acks) and never resolves, so reaching here
                // means some OTHER subsystem exited while the latch was engaged.
                // The node must stay up + observable regardless: do NOT abort-all —
                // keep marshal + `consensus`-RPC + epoch_manager serving
                // (epoch_manager already demoted to verify-only via the halt
                // latch). Park by awaiting the surviving handles so `run` does not
                // return (returning cancels the shutdown token → full teardown); they
                // resolve only on a real external shutdown (dpos.rs aborts this task).
                tracing::warn!(
                    subsystem = exit.0,
                    "SafetyHalt engaged — parking: marshal + consensus-RPC + epoch_manager stay \
                     alive (verify-only, observable); NOT aborting-all. Recovery is the L1 SP1 \
                     validity proof + governance."
                );
                park_supervisor(vec![
                    buffered_handle,
                    marshal_handle,
                    slasher_handle,
                    em_handle,
                    executor_handle,
                ])
                .await;
            }
            SupervisorAction::AbortAll => {
                buffered_handle.abort();
                executor_handle.abort();
                marshal_handle.abort();
                slasher_handle.abort();
                em_handle.abort();
            }
        }
    }

    /// Near-planeless FOLLOWER start (Phase 3). A non-validator OuterEngine
    /// (`signer_keypair: None`) runs marshal + executor + scheme provider + a
    /// gossip-idle `buffered` engine, fed by the cert-inlet — NOT the local BFT
    /// engine. It keeps exactly ONE plane piece: a minimal broadcast `Muxer` (so
    /// the buffered engine is alive to answer the marshal's buffer-first body
    /// lookup with `None`, after which either the verified-cache the inlet
    /// populated via `verified()` resolves the body locally, OR — for the gap
    /// between the cold-start floor and the upstream's live frontier — the
    /// [`UpstreamResolver`](crate::cert_inlet::UpstreamResolver) backfills it
    /// by-height from the cert upstream). It has NO vote/cert/resolver/marshal
    /// muxes (the resolver is upstream-backed, not p2p), NO `SharedBeaconPlane` /
    /// DkgActor / beacon oracle / signer, and DOES NOT start the slasher (a
    /// non-signer can never submit slashing). `epoch_manager` runs with
    /// `Option<Muxes>::None`, so `reconcile_roles` keeps it `Verifier` forever and
    /// `spawn_engine` is unreachable.
    pub fn start_follower<HS, HR, U>(
        self,
        broadcast_mux: SharedMux<HS, HR>,
        resolver_ctx: E,
        upstream: Option<U>,
    ) -> Handle<()>
    where
        E: Clone + Sync,
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
        U: crate::cert_follow::CertUpstream,
    {
        let mut this = self;
        spawn_cell!(
            this.context,
            this.run_follower(broadcast_mux, resolver_ctx, upstream)
                .await
        )
    }

    async fn run_follower<HS, HR, U>(
        self,
        broadcast_mux: SharedMux<HS, HR>,
        resolver_ctx: E,
        upstream: Option<U>,
    ) where
        E: Clone + Sync,
        HS: Sender<PublicKey = PublicKey>,
        HR: Receiver<PublicKey = PublicKey>,
        U: crate::cert_follow::CertUpstream,
    {
        // Register subchannel 0 on the ONE broadcast mux for the buffered engine.
        // NO marshal mux (the resolver is parked), NO vote/cert/resolver muxes
        // (the manager never spawns an engine).
        let broadcast = match broadcast_mux.lock().await.register(0).await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!(error = ?e, "follower broadcast mux register(0) failed — OuterEngine exiting");
                return;
            }
        };

        // The marshal resolver channel is LIVE on a follower. The inlet only
        // pre-caches bodies for the heights it ingests off the upstream's LIVE
        // stream — which starts at the upstream's CURRENT frontier, well above the
        // cold-start marshal floor (`landing − 2K`). The marshal dispatches to the
        // executor only CONTIGUOUSLY from `floor + 1`, so it must backfill the gap
        // `[floor+1 .. first_live_height]` (and any live-stream drops) by-height.
        // A follower has zero consensus-plane connectivity, so a peer resolver
        // would find nothing — instead the resolver pulls each missing height from
        // the cert UPSTREAM and delivers `(finalization, block)` back through this
        // `Handler`, which the marshal BLS-verifies before storing (the trustless
        // gate). With a `NoopResolver` here the gap never fills and the executor
        // stays idle forever (the cert-follow wedge). The `handler` wraps the
        // SENDER whose `marshal_rx` the actor consumes; both are held for the whole
        // follower lifetime so `resolver_rx` never closes.
        let (marshal_deliver_tx, marshal_rx) =
            mpsc::channel::<marshal_handler::Message<Digest>>(self.mailbox_size.max(1));
        // Parked clone of the sender: a `NoopResolver` (no-upstream config) does not
        // hold a `Handler`, so without this the only sender would drop and the
        // actor's `resolver_rx` would close → the marshal returns immediately. The
        // `Upstream` resolver holds its own `Handler` clone, so this is dead weight
        // there but harmless.
        let _parked_marshal_tx = marshal_deliver_tx.clone();
        let handler = marshal_handler::Handler::<Digest>::new(marshal_deliver_tx);
        let marshal_resolver = match upstream {
            Some(up) => crate::cert_inlet::FollowerResolver::Upstream(
                crate::cert_inlet::UpstreamResolver::new(
                    resolver_ctx,
                    up,
                    handler,
                    self.randomness.clone(),
                ),
            ),
            None => {
                crate::cert_inlet::FollowerResolver::Noop(crate::cert_inlet::NoopResolver::default())
            }
        };
        let marshal_chan = (marshal_rx, marshal_resolver);

        // Vote-backup is PARKED: a follower's manager only soft-enters and never
        // consumes catch-up hints, but `run` exits if its `vote_backup` receiver
        // closes — so hold the sender alive for the lifetime.
        let (_parked_vote_backup_tx, parked_vote_backup) = mpsc::channel(1);

        let mut em_handle = self.epoch_manager.start::<HS, HR>(None, parked_vote_backup);
        let mut buffered_handle = self.buffered.start(broadcast);
        let mut executor_handle = self.executor.start();
        let app_reporter: Reporters<marshal::Update<OrderBlock>, FluentApp<XC, A>, FeedSink> =
            Reporters::from((self.marshal_reporter_app, self.feed));
        let mut marshal_handle =
            self.marshal
                .start(app_reporter, self.buffer_mailbox, marshal_chan);
        // The slasher is built (cheap, no WAL traffic without a started actor) but
        // NOT started on a follower: dropping the unstarted actor omits it from the
        // supervisor and its WAL/reader never run.
        drop(self.slasher);

        // Supervisor: on first subsystem exit, abort the others. Held alive across
        // the select: `_parked_marshal_tx` + `_parked_vote_backup_tx` keep the
        // marshal resolver_rx / manager vote_backup open.
        let exit = tokio::select! {
            r = &mut buffered_handle => ("buffered", r),
            r = &mut executor_handle => ("executor", r),
            r = &mut marshal_handle => ("marshal", r),
            r = &mut em_handle => ("epoch_manager", r),
        };

        match exit.1 {
            Ok(()) => tracing::warn!(
                subsystem = exit.0,
                "follower subsystem exited cleanly (unexpected)"
            ),
            Err(e) => tracing::error!(subsystem = exit.0, error = ?e, "follower subsystem failed"),
        }

        match supervisor_action(&self.safety_halt) {
            SupervisorAction::ParkHalted => {
                // Phase-3 SafetyHalt on a follower (its executor derives+imports off
                // the inlet and can hit result divergence / EL Invalid): stop driving
                // reth but keep marshal + `consensus`-RPC alive. Same BACKSTOP as the
                // validator supervisor — the halted executor parks in-task and never
                // resolves, so this fires only on another subsystem's exit.
                tracing::warn!(
                    subsystem = exit.0,
                    "SafetyHalt engaged — parking follower: marshal + consensus-RPC stay alive; \
                     NOT aborting-all. Recovery is the L1 SP1 validity proof + governance."
                );
                park_supervisor(vec![
                    buffered_handle,
                    marshal_handle,
                    em_handle,
                    executor_handle,
                ])
                .await;
            }
            SupervisorAction::AbortAll => {
                buffered_handle.abort();
                executor_handle.abort();
                marshal_handle.abort();
                em_handle.abort();
            }
        }
    }
}

#[cfg(test)]
mod supervisor_tests {
    use super::{park_supervisor, supervisor_action, SupervisorAction};
    use crate::sync_metrics::{SafetyHalt, SyncMetrics, SyncReason};
    use commonware_runtime::{deterministic, Handle, Metrics as _, Runner as _, Spawner as _};
    use futures::FutureExt as _;
    use std::time::Duration;

    // The park must survive the handle that ALREADY resolved in the supervisor's
    // `select!`: awaiting it again re-polls a completed tokio oneshot, which panics
    // "called after complete" and tears the whole node down through
    // `with_catch_panics(true)`. Every subsystem takes a turn as the exiter because
    // the arm is reachable from each of them.
    #[test]
    fn park_supervisor_holds_a_resolved_handle_without_repolling_it() {
        let runner = deterministic::Runner::timed(Duration::from_secs(5));
        runner.start(|ctx| async move {
            for exiter in 0..5usize {
                let mut handles: Vec<Handle<()>> = (0..5usize)
                    .map(|i| {
                        ctx.with_label("subsystem").spawn(move |_| async move {
                            if i != exiter {
                                std::future::pending::<()>().await;
                            }
                        })
                    })
                    .collect();
                // Mirror the supervisor: its `select!` polls the handles by `&mut`
                // and returns the first that resolves.
                let _ = (&mut handles[exiter]).await;

                let mut park = park_supervisor(handles).boxed();
                assert!(
                    (&mut park).now_or_never().is_none(),
                    "park must not resolve (returning cancels the shutdown token)"
                );
            }
        });
    }

    // Fork-safety: a subsystem exit while the SafetyHalt latch is engaged PARKS
    // (marshal + consensus-RPC stay up, gauge raised) instead of abort-all; a
    // subsystem exit with NO halt engaged (a real crash) still ABORTS-ALL. The
    // supervisor must not have neutered the crash path.
    #[test]
    fn safety_halt_parks_but_a_real_crash_still_aborts_all() {
        let metrics = SyncMetrics::default();
        let halt = SafetyHalt::new(metrics.clone());

        // No halt engaged ⇒ a genuine subsystem crash ⇒ abort-all (unchanged).
        assert_eq!(supervisor_action(&halt), SupervisorAction::AbortAll);

        // Engaging the fork-safety latch flips the supervisor to park + raises the
        // alert gauge — the node stays up, verify-only, observable.
        halt.engage(SyncReason::ResultDivergence);
        assert_eq!(supervisor_action(&halt), SupervisorAction::ParkHalted);
        assert_eq!(metrics.degraded_value(SyncReason::ResultDivergence), 1);

        // A fresh, un-engaged latch still aborts — proving park is gated on the
        // latch, not a permanent supervisor change.
        assert_eq!(
            supervisor_action(&SafetyHalt::default()),
            SupervisorAction::AbortAll
        );
    }
}

#[cfg(test)]
mod resolver_routing_tests {
    use super::{is_finalized, marshal_handler};
    use crate::digest::Digest;
    use alloy_primitives::B256;
    use commonware_consensus::types::{Epoch, Height, Round, View};

    // Bug 12 routing policy: the Hybrid resolver sends `Finalized{height}` to the
    // upstream (by-height catch-up) and `Block`/`Notarized` to the consensus plane
    // (live-round repair). `is_finalized` is the single branch the delegation keys
    // on; pin it so a future edit can't silently reroute Finalized onto the plane
    // (reopening the 2026-06-22 out-of-committee wedge) or the live repair onto the
    // upstream (which cannot serve it).
    #[test]
    fn hybrid_routes_finalized_to_upstream_others_to_plane() {
        let finalized = marshal_handler::Request::<Digest>::Finalized {
            height: Height::new(7),
        };
        let block = marshal_handler::Request::<Digest>::Block(Digest(B256::ZERO));
        let notarized = marshal_handler::Request::<Digest>::Notarized {
            round: Round::new(Epoch::new(1), View::new(9)),
        };
        assert!(is_finalized(&finalized), "Finalized routes to the upstream");
        assert!(!is_finalized(&block), "Block routes to the plane");
        assert!(!is_finalized(&notarized), "Notarized routes to the plane");
    }
}
