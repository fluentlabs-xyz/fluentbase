//! DPoS layer launcher — assembles 03 (staking-reader), 04 (consensus),
//! and 05 (p2p) given operator keys, reth handles, and config. Spawned
//! by the host adapter at `crates/node/src/dpos.rs`.

use crate::{
    application::{
        derive_with_visibility_retry, BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder,
        ExecutedChain, OrderingAssembler,
    },
    order_block::{anchor_order_block, K},
    scheme::epoch_committee_from_snapshot,
    slasher::actor::{SharedCacheFallback, SlasherTxSink, StaleEpochFallback},
    timeouts::ConsensusTimeouts,
    OuterBuilder,
};
use alloy_consensus::Header;
use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::types::{Epoch, Height};
use commonware_cryptography::Signer;
use commonware_p2p::{authenticated::discovery::Bootstrapper, Ingress};
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use commonware_storage::{
    archive::{Archive as _, Identifier},
    metadata::{self, Metadata},
};
use commonware_utils::sequence::U64;
use eyre::{ensure, eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::{
    fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_verifier, PeerPubkey,
};
use fluentbase_p2p::{FluentP2P, FluentP2PConfig};
use fluentbase_staking_reader::{
    reader::StakingReaderConfig, EpochTransition, RethStakingStateReader, TransitionOutcome,
    ValidatorSetCache,
};
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
use reth_primitives_traits::SealedBlock;
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::{
    net::SocketAddr,
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Threshold for consecutive `on_finalized` errors before initiating shutdown.
/// At 1 block/sec finalization, 3 = ~3 seconds tolerance. Survives transient
/// errors (single bad read, reorg edge); fails fast on persistent (disk full,
/// chain config error, DB corruption). Production posture.
const MAX_CONSECUTIVE_ON_FINALIZED_ERRORS: u32 = 3;

/// Partition prefix for the commonware marshal's durable storage (finalizations,
/// finalized blocks, application-metadata). Shared between the cold-start
/// discriminator peek (`read_consensus_archive_last_finalized`) and the marshal
/// itself (`OuterBuilder.partition_prefix`) so the two never drift.
const MARSHAL_PARTITION_PREFIX: &str = "consensus_marshal";

/// Reth handles needed by the DPoS layer. The host adapter at
/// `crates/node/src/dpos.rs` assembles this from `FullNode<N, AddOns>`;
/// `transaction_pool`, `chain_spec`, and `data_dir` are intentionally
/// absent — `slasher_sink` arrives pre-built via `DposLayerConfig` (so
/// the host owns the `reth-transaction-pool` trait bounds), `chain_spec`
/// reduces to its only used field `chain_id`, and `data_dir` is set
/// host-side in `spawn_dpos` before `runner.start()`.
pub struct RethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    /// Disk-loaded canonical state snapshot. Reth's
    /// `BlockchainProvider::with_latest` populates `finalized_block` /
    /// `safe_block` from `ChainState::LastFinalizedBlock` during node
    /// init, so on a graceful-shutdown restart
    /// `get_finalized_num_hash()` returns `Some(disk_finalized.num_hash())`.
    /// Kept as a struct field rather than a trait method because
    /// `canonical_in_memory_state()` is a concrete inherent on
    /// `BlockchainProvider<N>`, not exposed via any reth provider trait.
    pub canonical_state: reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    /// Pristine-network fallback for when
    /// `canonical_state.get_finalized_num_hash()` returns `None`.
    pub genesis_hash: B256,
}

/// Cold-start `(finalized_num, finalized_hash, head_num, head_hash)` derived
/// purely from reth's `canonical_state` + `genesis_hash` (the non-migration
/// path). Reth's `BlockchainProvider::with_latest` repopulates
/// `canonical_state.finalized_block` on a graceful-restart, so
/// `get_finalized_num_hash()` returns the disk finalized; the genesis fallback
/// covers a pristine network (no FCU yet). Extracted from [`DposLayer::launch`]
/// so the cold-start arithmetic is unit-tested against this production code
/// rather than a copy.
pub fn derive_cold_start_heights(
    canonical_state: &reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    genesis_hash: B256,
) -> (u64, B256, u64, B256) {
    let finalized = canonical_state
        .get_finalized_num_hash()
        .unwrap_or_else(|| alloy_eips::BlockNumHash::new(0, genesis_hash));
    let head = canonical_state.chain_info();
    (
        finalized.number,
        finalized.hash,
        head.best_number,
        head.best_hash,
    )
}

/// Bounded wait for reth to hold the DPoS activation block before adopting it as
/// the fresh-migration consensus anchor; returns the block's local-canonical
/// hash. Covers reth still replaying MDBX on restart. A timeout is fatal: the
/// sequencer must have finalized the activation block before DPoS starts. No
/// operator-hash compare — the activation height comes from the on-chain
/// `ChainConfig.dposActivationBlock` and the hash is local-canonical at a
/// finalized height (every honest node derives the same hash).
pub(crate) async fn wait_for_activation_block<Provider>(
    ctx: &Context,
    provider: &Provider,
    activation: u64,
) -> eyre::Result<B256>
where
    Provider: BlockHashReader,
{
    // Generous enough to cover reth finishing its MDBX/static-file replay on a
    // cold restart even under multi-node resource contention (a first-run devnet
    // brings several reth instances up at once, and one can lose a tight startup
    // race while the activation block IS present — it's just not queryable yet).
    // 30s was too tight and produced a spurious fatal "activation block missing".
    const ANCHOR_WAIT: Duration = Duration::from_secs(120);
    let deadline = ctx.current() + ANCHOR_WAIT;
    loop {
        if let Some(hash) = provider
            .block_hash(activation)
            .wrap_err("provider.block_hash failed during activation-block wait")?
        {
            tracing::info!(height = activation, hash = ?hash, "DPoS activation block present in reth");
            return Ok(hash);
        }
        if ctx.current() >= deadline {
            return Err(eyre!(
                "reth does not have DPoS activation block {activation} after {ANCHOR_WAIT:?}; \
                 wait for the sequencer to produce and persist it before starting DPoS"
            ));
        }
        ctx.sleep(Duration::from_secs(2)).await;
    }
}

/// Peek the marshal's last consensus-finalized height from its durable
/// application-metadata store WITHOUT building the marshal/engine — the
/// restart-vs-fresh-migration discriminator. An empty store (fresh migration)
/// returns 0; a populated store (restart, already migrated) returns the last
/// DPoS-finalized height so the cold-start resumes at the correct epoch.
///
/// Reads the SAME `{partition_prefix}-application-metadata` Metadata store and
/// key that commonware `MarshalActor::init` returns as `last_processed_height`
/// (monorepo `consensus/src/marshal/core/actor.rs:305-317`), so the value is
/// byte-identical to the one the executor-seed path already consumes. The peek
/// opens the store, reads, and drops it before `MarshalActor::init` re-opens it.
pub(crate) async fn read_consensus_archive_last_finalized(
    ctx: &Context,
    partition_prefix: &str,
) -> eyre::Result<u64> {
    // Wire-format invariant: must match commonware marshal `core/actor.rs:58`
    // `const LATEST_KEY: U64 = U64::new(0xFF)` (a private const there). It is a
    // storage-layout constant pinned with the commonware rev in `Cargo.lock`.
    const LATEST_KEY: U64 = U64::new(0xFF);
    let metadata: Metadata<Context, U64, Height> = Metadata::init(
        ctx.with_label("cold_start_archive_peek"),
        metadata::Config {
            partition: format!("{partition_prefix}-application-metadata"),
            codec_config: (),
        },
    )
    .await
    .wrap_err("opening marshal application-metadata for cold-start discriminator")?;
    Ok(metadata
        .get(&LATEST_KEY)
        .copied()
        .unwrap_or(Height::zero())
        .get())
}

/// Crash-survivor cold-start recovery: reth is missing the
/// consensus-finalized block at `target` (an ungraceful crash lost reth's
/// unflushed tail while the marshal persisted the finalization). Read the missing
/// block(s) from the marshal's own `finalized_blocks` archive and `new_payload`
/// them into reth, walking ancestors oldest-ward until reth reconnects; return the
/// recovered `target`'s local hash. Standalone archive open (before the engine is
/// built), like the metadata peek — dropped before `MarshalActor::init` re-opens it.
async fn recover_finalized_tail_into_reth<Provider, BeaconEngine, D>(
    ctx: &Context,
    beacon_engine: &BeaconEngine,
    provider: &Provider,
    deriver: &D,
    target: u64,
) -> eyre::Result<B256>
where
    Provider: BlockHashReader,
    BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived>,
    D: DerivedBlockBuilder,
{
    // An ungraceful crash loses only reth's unflushed tail (typically 1-2 blocks).
    // A larger gap is real EL data loss, not a recoverable flush race.
    const MAX_COLD_RECOVER: u64 = 64;

    let archive = crate::outer::init_finalized_blocks_archive(ctx, MARSHAL_PARTITION_PREFIX).await;
    // Re-derived heights must use the SAME seed peers used (recovered from the
    // finalization cert), not the gated fallback — else a restart diverges. Open
    // the finalizations archive standalone (like `archive` above) to read it.
    let finalizations = {
        let page_cache = commonware_runtime::buffer::paged::CacheRef::from_pooler(
            ctx,
            crate::outer::PAGE_CACHE_PAGE_SIZE,
            crate::outer::PAGE_CACHE_CAPACITY,
        );
        crate::outer::init_finalizations_archive(ctx, MARSHAL_PARTITION_PREFIX, page_cache).await
    };

    // Find the lowest missing height (the reconnect point is its parent). The
    // archive stores ORDERING artifacts, so recovery re-derives each missing
    // block oldest-first — same deterministic function as live execution.
    let mut lowest = target;
    while lowest > 0
        && provider
            .block_hash(lowest - 1)
            .wrap_err("provider.block_hash during crash-survivor recovery")?
            .is_none()
    {
        if target - lowest >= MAX_COLD_RECOVER {
            return Err(eyre!(
                "crash-survivor recovery: reth missing > {MAX_COLD_RECOVER} blocks below \
                 finalized {target} — real EL data loss; re-sync the EL disk from a snapshot"
            ));
        }
        lowest -= 1;
    }
    let mut parent_hash = provider
        .block_hash(lowest.saturating_sub(1))
        .wrap_err("provider.block_hash at recovery reconnect point")?
        .ok_or_else(|| {
            eyre!(
                "crash-survivor recovery: no reconnect parent below height {lowest}; \
                 re-sync the EL disk from a snapshot"
            )
        })?;
    for h in lowest..=target {
        let order = archive
            .get(Identifier::Index(h))
            .await
            .map_err(|e| eyre!("reading marshal finalized_blocks at height {h}: {e}"))?
            .ok_or_else(|| {
                eyre!(
                    "crash-survivor recovery: marshal finalized_blocks has no block at height \
                     {h} (gap exceeds marshal retention); re-sync the EL disk from a snapshot"
                )
            })?;
        let seed = finalizations
            .get(Identifier::Index(h))
            .await
            .ok()
            .flatten()
            .and_then(|fin| {
                fin.certificate
                    .seed()
                    .map(|signature| crate::beacon::types::Seed {
                        target_round: fin.proposal.round,
                        signature,
                    })
            });
        let derived = derive_with_visibility_retry(ctx, deriver, &order, parent_hash, seed)
            .await
            .wrap_err("crash-survivor recovery derivation failed")?;
        parent_hash = derived.evm_hash();
        let status = beacon_engine
            .import_derived(derived)
            .await
            .wrap_err("crash-survivor recovery import failed")?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL rejected recovered finalized block {h}: {status:?}"
        );
        // Per-block FCU, awaited — the SAME visibility sync point the live
        // executor relies on. An InsertExecuted import "adds to canonical
        // chain" but header-by-hash reads do NOT see the block until an FCU
        // lands (observed unbounded, not ms-scale: a 10s retry expired
        // against it), so the next iteration's parent read would fail
        // without this. The retry above still covers the devp2p-concurrent
        // import case, where the canonicalizer is not us.
        let resp = beacon_engine
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: parent_hash,
                safe_block_hash: parent_hash,
                finalized_block_hash: parent_hash,
            })
            .await
            .wrap_err("crash-survivor recovery per-block FCU failed")?;
        ensure!(
            resp.is_valid(),
            "EL rejected crash-survivor recovery FCU at {h}: {:?}",
            resp.payload_status
        );
    }
    drop(archive); // release so MarshalActor::init can re-open the same partition.

    provider
        .block_hash(target)
        .wrap_err("provider.block_hash after crash-survivor recovery")?
        .ok_or_else(|| {
            eyre!("crash-survivor recovery: block {target} still missing from reth after replay")
        })
}

/// Operator-supplied per-launch configuration. Keys + JSON-parsed
/// configs arrive pre-loaded (the host crate owns filesystem syscalls
/// and permission checks); the slasher transport arrives pre-built
/// because `PoolTxSink<P, Provider>` carries concrete
/// `reth-transaction-pool` trait bounds that can't compile in this crate.
pub struct DposLayerConfig<D, XC, A> {
    pub bls_keypair: ValidatorBlsKeypair,
    pub peer_keypair: commonware_cryptography::ed25519::PrivateKey,
    pub slasher_sink: Arc<dyn SlasherTxSink>,
    pub staking_config: StakingReaderConfig,
    pub bootstrappers: Vec<Bootstrapper<PeerPubkey>>,
    pub p2p: P2pParams,
    /// OrderBlock → derived-EVM-block execution (node-built over reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-built over the reth provider).
    pub executed: XC,
    /// Pool-backed ordering assembly (node-built — pool trait bounds live there).
    pub assembler: Arc<A>,
    /// This node's own proposals only (agreed data once embedded).
    pub fee_recipient: Address,
    pub target_gas_limit: u64,
    /// Cert-feed sink (node-built): wired as the marshal's second
    /// application-`Reporter` so a node-side feed actor can serve the
    /// `consensus` RPC. `None` for nodes that don't serve the cert feed.
    pub feed: Option<crate::feed_sink::FeedSink>,
    /// Unified-supervisor promotion (NOT operator-set): the supervisor's own
    /// follower phase just verified the chain up to the EL tip, so the
    /// discriminator heuristics are bypassed entirely — anchor at the EL
    /// finalized tip even over a populated-but-stale consensus archive (the
    /// raises-only marshal floor masks the stale prefix). The heuristics
    /// exist only because a restarted PROCESS loses this context; the
    /// in-process supervisor has it.
    pub promotion: bool,
    /// Mode signals to the unified supervisor (rotation-out → demote).
    /// `None` = legacy `--dpos` (no supervisor listening).
    pub mode_events: Option<tokio::sync::mpsc::UnboundedSender<ModeEvent>>,
    /// Randomness-beacon launch material (devnet bootstrap). `None` = no beacon
    /// (gated fallback `prev_randao = order.digest()`). When present, the layer
    /// threads the threshold key into every per-epoch combined consensus scheme,
    /// so each vote carries the seed partial and the seed is recovered from the
    /// finalization certificate.
    pub beacon: Option<BeaconLaunch>,
}

/// Threshold randomness-beacon material handed to the DPoS layer at launch
/// (devnet bootstrap; the live DKG actor is phased). Threaded into the combined
/// consensus scheme; the deriver reads the recovered seed from the certificate.
pub struct BeaconLaunch {
    /// This node's DKG share, or `None` for a verifier-only beacon participant.
    pub share: Option<commonware_cryptography::bls12381::primitives::group::Share>,
    /// The public polynomial (`.public()` is `PK_epoch`).
    pub sharing: commonware_cryptography::bls12381::primitives::sharing::Sharing<
        commonware_cryptography::bls12381::primitives::variant::MinSig,
    >,
}

/// Mode signal from the per-epoch engine construction to the unified
/// supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeEvent {
    /// The signer keypair is absent from `committee[epoch]` — the validator
    /// was rotated out; the supervisor demotes the node to the follower
    /// plane (the just-built verifier engine is aborted with the stack).
    RotatedOut { epoch: u64 },
}

/// Cold-start kind resolved from durable state. Pure function of the inputs
/// so the decision is unit-testable without a node. The unified supervisor's
/// `Promotion` bypasses this resolver entirely (it KNOWS its follower phase
/// verified the chain — the heuristics below exist only because a restarted
/// process loses that context).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColdStartKind {
    /// Empty archive, EL at/inside epoch 0: anchor at the activation block.
    FreshMigration,
    /// Populated archive: resume at its finalized height (real consensus
    /// state always wins).
    Restart,
    /// Unified-supervisor promotion: anchor at the EL finalized tip
    /// UNCONDITIONALLY (a stale archive is masked by the raises-only marshal
    /// floor).
    Promotion,
}

fn resolve_cold_start_kind(
    archive_finalized: u64,
    activation: u64,
    interval: u32,
    cs_finalized: u64,
) -> eyre::Result<ColdStartKind> {
    if archive_finalized > activation {
        return Ok(ColdStartKind::Restart);
    }
    if cs_finalized >= activation + interval as u64 {
        return Err(eyre!(
            "EL is past epoch 0 (finalized {cs_finalized} >= activation {activation} + interval \
             {interval}) with an empty consensus archive; refusing to anchor DPoS on a state of \
             unknown provenance. Run unified mode (--dpos.follower-upstream) so the node \
             verifies-and-promotes itself, or restore the consensus archive."
        ));
    }
    Ok(ColdStartKind::FreshMigration)
}

/// Peek the consensus marshal archive's last-finalized height — the same value
/// the cold-start discriminator reads. Returns 0 when the archive is
/// empty/absent (no consensus state ever persisted). The unified supervisor's
/// entry rule uses this to avoid choosing signer-first for an in-committee node
/// that has no consensus state to resume (which would otherwise hit the
/// `resolve_cold_start_kind` "empty archive + EL past epoch 0" fatal instead of
/// following to build the archive first).
pub async fn peek_consensus_archive_last_finalized(ctx: &Context) -> eyre::Result<u64> {
    read_consensus_archive_last_finalized(ctx, MARSHAL_PARTITION_PREFIX).await
}

pub struct P2pParams {
    pub listen: SocketAddr,
    pub dialable: Option<SocketAddr>,
}

pub struct DposLayerHandle {
    pub consensus_handle: Handle<()>,
    pub network_handle: Handle<()>,
    /// Marshal mailbox clone for the node-side cert feed/RPC (by-height
    /// `get_finalization`+`get_block`). The node calls `feed_handle.set_marshal`
    /// with this once `launch` returns — keeping node types out of consensus.
    pub cert_mailbox: crate::outer::MarshalMailbox,
}

/// Namespace type for the launch entry point.
pub struct DposLayer;

impl DposLayer {
    /// Launch the DPoS layer end-to-end: build 03 reader+cache+EpochTransition,
    /// 05 p2p network, 04 OuterEngine; perform cold-start; spawn forwarder
    /// + outer + network; return their `Handle<()>`s for the host to supervise.
    ///
    /// Caller (the host adapter at `crates/node/src/dpos.rs`) is responsible
    /// for the `select!` supervisor over `shutdown` + the two returned
    /// handles. Caller also performs filesystem key loading
    /// and `PoolTxSink` construction before calling.
    #[allow(clippy::too_many_arguments)]
    pub async fn launch<Provider, EvmConfig, BeaconEngine, D, XC, A>(
        ctx: Context,
        reth: RethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: DposLayerConfig<D, XC, A>,
        shutdown: CancellationToken,
    ) -> eyre::Result<DposLayerHandle>
    where
        Provider: BlockReader<Block = RethBlock>
            + BlockHashReader
            + BlockNumReader
            + BlockIdReader
            + StateProviderFactory
            + HeaderProvider<Header = Header>
            + Clone
            + Send
            + Sync
            + 'static,
        EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
        BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
        D: DerivedBlockBuilder,
        XC: ExecutedChain,
        A: OrderingAssembler,
    {
        let DposLayerConfig {
            bls_keypair,
            peer_keypair,
            slasher_sink,
            staking_config,
            bootstrappers,
            p2p: P2pParams { listen, dialable },
            deriver,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            feed,
            promotion,
            mode_events,
            beacon,
        } = cfg;

        let RethHandle {
            provider,
            evm_config,
            beacon_engine_handle,
            chain_id,
            canonical_state,
            genesis_hash,
        } = reth;

        // Build the staking-reader layer: reader + cache + EpochTransition.
        let staking_address = staking_config.staking_address;
        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        // Dedicated reader instance for the slasher (NOT shared with ET).
        // `RethStakingStateReader` is not `Clone`; ctor args are
        // already cloned at the call sites. Each instance lazy-inits its own
        // `OnceLock<u32>` epoch interval/undelegate cache on first call —
        // negligible (~2 extra reads at startup).
        let reader_for_slasher = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        // Clones for the live-DKG beacon actor's committee resolver — taken before
        // `reader` is moved into `EpochTransition` below. `RethStakingStateReader`
        // is `Clone`; `provider` (BlockNumReader + BlockHashReader) resolves the
        // current finalized EVM hash the committee snapshot reads at.
        let reader_for_dkg = reader.clone();
        let provider_for_dkg = provider.clone();

        // Reth's `BlockchainProvider::with_latest` populates
        // `canonical_state.finalized_block` from
        // `ChainState::LastFinalizedBlock` during node init, so on a
        // graceful-shutdown restart `get_finalized_num_hash()` returns
        // `Some(disk_finalized.num_hash())`. The genesis fallback
        // handles the pristine-network case (no FCU yet).
        let (cs_finalized, cs_finalized_hash, head_num, head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

        // Activation origin + epoch length, read EARLY (at the reth-restored
        // finalized hash) — the cold-start discriminator below needs them before
        // `initial_epoch`. `dposActivationBlock` is immutable and
        // `epochBlockInterval` is governance-stable across the short
        // migration/restart window, so reading at `cs_finalized_hash` matches
        // reading at the resumed height.
        let dpos_activation_block = reader.dpos_activation_block(cs_finalized_hash)?;
        let interval = reader.epoch_block_interval(cs_finalized_hash)?;
        let epoch_length_blocks =
            NonZeroU64::new(interval as u64).ok_or_eyre("epoch_block_interval must be > 0")?;

        // Cold-start discriminator (restart vs fresh migration; the unified
        // supervisor's Promotion bypasses it — see resolve_cold_start_kind).
        // The marshal's durable application-metadata is the signal: an empty
        // store (height <= activation) is a fresh Tempo→DPoS migration —
        // unless the EL overshot epoch 0, which is fatal (state of unknown
        // provenance). A populated store is a restart of an already-migrated
        // node, which MUST resume at its real finalized height so the scheme
        // cascade starts at the correct epoch.
        let archive_finalized =
            read_consensus_archive_last_finalized(&ctx, MARSHAL_PARTITION_PREFIX).await?;
        let kind = if promotion {
            ColdStartKind::Promotion
        } else {
            resolve_cold_start_kind(
                archive_finalized,
                dpos_activation_block,
                interval,
                cs_finalized,
            )?
        };
        let (latest_finalized, latest_finalized_hash) = if kind == ColdStartKind::Promotion {
            info!(
                anchor = cs_finalized,
                archive_finalized,
                "PROMOTION cold-start: supervisor follower phase verified the chain; \
                 anchoring at the EL finalized tip"
            );
            (cs_finalized, cs_finalized_hash)
        } else if kind == ColdStartKind::FreshMigration {
            // FRESH MIGRATION: anchor ≡ block@dposActivationBlock; wait for reth
            // to hold it, hash derived locally (canonical at a finalized height).
            // Checkpoint-provisioned EL-ahead start is deferred to Phase 2/β — a
            // node-local EL head as genesis would diverge cross-node.
            let hash = wait_for_activation_block(&ctx, &provider, dpos_activation_block).await?;
            (dpos_activation_block, hash)
        } else {
            // RESTART (already migrated): resume at the consensus archive's
            // finalized height.
            let hash = match provider.block_hash(archive_finalized)? {
                Some(hash) => hash,
                None => {
                    // CRASH SURVIVOR: an ungraceful crash lost reth's
                    // unflushed finalized tail while the marshal persisted the
                    // finalization (the two stores flush independently). reth is
                    // behind the consensus archive. Recover the missing block(s)
                    // from the marshal's OWN finalized_blocks archive into reth —
                    // the cold-start analog of the executor gap-heal, and how tempo
                    // backfills marshal→reth. fluentbase needs this at cold-start
                    // (not just in the executor backfill) because the committee
                    // read at `latest_finalized_hash` and the genesis read both
                    // require reth to hold the resume block. The later executor
                    // backfill then becomes a no-op.
                    recover_finalized_tail_into_reth(
                        &ctx,
                        &beacon_engine_handle,
                        &provider,
                        &deriver,
                        archive_finalized,
                    )
                    .await?
                }
            };
            (archive_finalized, hash)
        };

        // Read AFTER the crash-survivor recovery above: recovery imports the
        // lost reth tail, and a pre-recovery snapshot would make the executor
        // backfill re-derive exactly those blocks (idempotent but wasted V).
        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head block number at startup")?;

        tracing::info!(
            last_execution_finalized_height,
            archive_finalized,
            ?kind,
            finalized = latest_finalized,
            finalized_hash = ?latest_finalized_hash,
            head_num,
            head_hash = ?head_hash,
            "DPoS init: cold-start discriminator resolved"
        );

        // Clean-halt migration invariant: the pre-DPoS sequencer is production-gated
        // at `dposActivationBlock` (bins/fluent launcher), so on a fresh migration
        // reth's canonical head MUST already equal the activation anchor — there is
        // no orphaned Tempo tail to reconcile. A mismatch means the gate did not run
        // (mis-set chain-config, an ungated node, or a hand-rolled migration): fail
        // loud at cold-start rather than wedge silently in the executor ancestor-FCU
        // guard.
        if kind == ColdStartKind::FreshMigration {
            ensure!(
                head_hash == latest_finalized_hash,
                "fresh migration but reth head {head_num} ({head_hash:?}) != activation \
                 anchor {latest_finalized} ({latest_finalized_hash:?}); the Tempo sequencer \
                 was not production-gated at dposActivationBlock — refusing to anchor DPoS \
                 on an orphaned tail"
            );
        }
        let (initial_head_num, initial_head_hash) = (head_num, head_hash);

        let undelegate = reader.undelegate_period(latest_finalized_hash)?;
        let retention =
            undelegate as u64 + fluentbase_staking_reader::reader::EPOCH_COMMITTEE_RETENTION_MARGIN;
        let initial_epoch_u64 = fluentbase_staking_reader::reader::epoch_of_block(
            latest_finalized,
            interval,
            dpos_activation_block,
        );

        // Enforce Rust ↔ Solidity invariant
        //   `ChainConfig.activeValidatorsLength <= fluentbase_p2p::MAX_COMMITTEE_SIZE`.
        let active_validators_length = reader
            .active_validators_length(latest_finalized_hash)
            .wrap_err("failed reading ChainConfig.activeValidatorsLength")?;
        if (active_validators_length as u64) > fluentbase_p2p::constants::MAX_COMMITTEE_SIZE {
            return Err(eyre!(
                "ChainConfig.activeValidatorsLength ({}) exceeds \
                 fluentbase_p2p::constants::MAX_COMMITTEE_SIZE ({}). Rust ↔ Solidity \
                 cap drift detected — bump MAX_COMMITTEE_SIZE in \
                 crates/p2p/src/constants.rs AND MAX_ACTIVE_VALIDATORS in \
                 solidity-contracts/contracts/staking/ChainConfig.sol in the SAME PR, \
                 then redeploy/upgrade.",
                active_validators_length,
                fluentbase_p2p::constants::MAX_COMMITTEE_SIZE,
            ));
        }

        info!(
            chain_id,
            interval,
            retention,
            max_committee_size = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE,
            active_validators_length,
            initial_epoch = initial_epoch_u64,
            latest_finalized,
            head_num,
            "DPoS startup config"
        );

        // Pre-cold-start reads: snapshot for initial scheme registration BEFORE
        // the reader is moved into EpochTransition.
        let initial_snapshot =
            reader.epoch_committee_snapshot(initial_epoch_u64, latest_finalized_hash)?;
        if initial_snapshot.validators.is_empty() {
            eyre::bail!(
                "Staking contract returned empty committee for epoch {initial_epoch_u64} \
                 (read at finalized block {latest_finalized}). \
                 Run commitEpochCommittee(address[]) on the staking contract \
                 before launching DPoS validators."
            );
        }

        // Wrap the cache in Arc<Mutex<>> so two consumers can read/write
        // it: `EpochTransition` (read+write on the boundary path) and the
        // slasher (read-only via `SharedCacheFallback`).
        let cache = Arc::new(Mutex::new(
            ValidatorSetCache::init(ctx.with_label("dpos_cache"))
                .await
                .wrap_err("failed initializing ValidatorSetCache")?,
        ));
        let slasher_stale_fallback: Arc<dyn StaleEpochFallback> =
            Arc::new(SharedCacheFallback(cache.clone()));

        // Build the p2p layer: FluentP2P + handles.
        let dialable_addr = dialable.unwrap_or(listen);
        info!(
            count = bootstrappers.len(),
            chain_id, "DPoS bootstrappers handed in by host"
        );
        let p2p_cfg = FluentP2PConfig {
            crypto: peer_keypair.clone(),
            chain_id,
            listen,
            dialable: Ingress::Socket(dialable_addr),
            bootstrappers,
        };
        let (p2p, handles) = FluentP2P::build(ctx.clone(), p2p_cfg);
        let network_handle = p2p.start();

        // Bridge channel: boundary triggers from EpochTransition queue here;
        // a forwarder task (spawned after build) drains bridge_rx →
        // outer_boundary_tx. Built BEFORE EpochTransition so boundary_tx is
        // wired at construction — eliminates the post-build
        // set_boundary_sender race window.
        let (bridge_tx, mut bridge_rx) =
            mpsc::channel::<(u64, fluentbase_staking_reader::reader::ValidatorSetSnapshot)>(64);

        // Wire staking-reader ↔ p2p: EpochTransition consumes the Oracle as PeerSetSink.
        let provider_for_et = provider.clone();
        let mut epoch_transition = EpochTransition::new(
            reader,
            cache,
            handles.oracle.clone(),
            fluentbase_p2p::constants::MAX_REGISTRY_PEER_SET as usize,
            Some(bridge_tx.clone()),
            Arc::new(move |n| provider_for_et.block_hash(n).ok().flatten()),
            K,
        );

        // Cold-start: read current finalized committee, track once.
        epoch_transition
            .cold_start(latest_finalized_hash, latest_finalized)
            .await
            .wrap_err("epoch_transition cold_start failed")?;
        info!(
            epoch = initial_epoch_u64,
            "DPoS cold_start complete; peer set tracked"
        );

        // Compute the consensus genesis Block.
        //
        // `latest_finalized` is 0 for pristine cold-start (no FCU yet — canonical
        // state empty → falls to `BlockNumHash::new(0, genesis_hash)` at dpos.rs:220)
        // and N for Tempo→DPoS migration (Tempo's last finalised height read from
        // disk by reth's `BlockchainProvider::with_latest`).
        //
        // For migration, anchoring the consensus genesis at block N (rather than
        // the fluent chain genesis at height 0) makes Simplex voter cache
        // `set_genesis(hash_N)` so view 1's `context.parent = (View::zero(), hash_N)`
        // matches the proposer's block.parent = hash_N. `fetch_parent`'s identity
        // short-circuit then returns the synthetic genesis (= block N),
        // `validate_block` passes, and `application.verify(block_N+1)` proceeds
        // to reth `new_payload` against MDBX-loaded state(hash_N).
        let genesis_unsealed = provider
            .block_by_number(latest_finalized)
            .map_err(|e| {
                eyre!("consensus genesis block read at height {latest_finalized} failed: {e}")
            })?
            .ok_or_else(|| {
                eyre!(
                    "consensus genesis block missing from MDBX at height {latest_finalized} \
                 (canonical_state.finalized claimed it exists). Graceful shutdown must \
                 persist this block before DPoS restart."
                )
            })?;
        let genesis_sealed: SealedBlock<RethBlock> = SealedBlock::seal_slow(genesis_unsealed);
        // F-type: the ordering-chain genesis is the deterministic anchor
        // artifact (its `result` field binds the anchor's EVM hash, so the
        // weak-subjectivity binding of the old executed-block genesis is
        // preserved). Every node computes the identical artifact, so Simplex
        // `set_genesis(digest)` matches view 1's parent on all nodes.
        let genesis_block = anchor_order_block(&genesis_sealed);

        // Move EpochTransition into Arc<Mutex<_>> so the boundary_hook
        // closure can call back into it from any thread.
        let et_arc = Arc::new(Mutex::new(epoch_transition));

        // Boundary hook: fires for every `Update::Block`. Spawns
        // fire-and-forget via `ctx.spawn` (NOT `tokio::spawn`, which would
        // depend on the implicit `tokio::Handle::current()` contract under
        // commonware-tokio).
        // Finalized-height tap for the live-DKG beacon actor: the boundary hook
        // fires for every finalized `Update::Block`, so it is the actor's epoch
        // clock (seal deadline + transition detection). Best-effort `try_send` —
        // a full/closed channel just means the actor missed a tick, recovered on
        // the next one (the ceremony window spans ~all of E-1).
        let (dkg_height_tx, dkg_height_rx) = mpsc::channel::<u64>(256);

        let consecutive_errors = Arc::new(AtomicU32::new(0));
        let shutdown_for_hook = shutdown.clone();
        let et_for_hook = et_arc.clone();
        let ctx_for_hook = ctx.with_label("boundary_hook");
        let errors_for_hook = consecutive_errors.clone();
        let dkg_height_tx_for_hook = dkg_height_tx.clone();
        let boundary_hook: Arc<dyn Fn(crate::order_block::OrderBlock) + Send + Sync> =
            Arc::new(move |block: crate::order_block::OrderBlock| {
                let _ = dkg_height_tx_for_hook.try_send(block.height);
                let et = et_for_hook.clone();
                let ctx_task = ctx_for_hook.clone();
                let shutdown = shutdown_for_hook.clone();
                let errors = errors_for_hook.clone();
                let number = block.height;
                // The old BlockNotFound retry loop is gone: committee reads
                // now resolve at the result-final height (number − K) inside
                // EpochTransition; an unresolved read is Intra + a pending
                // boundary that replays on the next delivery — no race with
                // the executor's import remains.
                drop(ctx_task.spawn(move |ctx_inner| async move {
                    // Re-poke loop: a parked boundary replays only on the next
                    // on_finalized call, and during epoch catch-up the parked
                    // boundary IS the last deliverable block — without the
                    // retry no further delivery would ever trigger the replay
                    // (catch-up deadlock). Bounded by execution progress: the
                    // executor has the boundary's range queued, so the park
                    // clears within the derive backlog (~ack window).
                    let mut retries = 0u32;
                    loop {
                        let outcome = {
                            let mut et_guard = et.lock().await;
                            et_guard.on_finalized(number).await
                        };
                        match outcome {
                            Ok(TransitionOutcome::EpochAdvanced(_)) => {
                                errors.store(0, Ordering::Relaxed);
                            }
                            Ok(TransitionOutcome::Intra) => {}
                            Err(e) => {
                                let count = errors.fetch_add(1, Ordering::Relaxed) + 1;
                                error!(
                                    block_number = number,
                                    consecutive_errors = count,
                                    error = ?e,
                                    "epoch_transition.on_finalized failed"
                                );
                                if count >= MAX_CONSECUTIVE_ON_FINALIZED_ERRORS {
                                    error!(
                                        count,
                                        threshold = MAX_CONSECUTIVE_ON_FINALIZED_ERRORS,
                                        "exceeded on_finalized error threshold; initiating shutdown"
                                    );
                                    shutdown.cancel();
                                }
                                break;
                            }
                        }
                        let parked = { et.lock().await.has_pending_boundary() };
                        if !parked {
                            break;
                        }
                        if retries
                            >= fluentbase_staking_reader::epoch_transition::PENDING_RETRY_LIMIT
                        {
                            error!(
                                block_number = number,
                                "pending boundary did not clear within the retry window"
                            );
                            break;
                        }
                        retries += 1;
                        ctx_inner
                            .sleep(
                                fluentbase_staking_reader::epoch_transition::PENDING_RETRY_BACKOFF,
                            )
                            .await;
                    }
                }));
            });

        let me = peer_keypair.public_key();
        info!(peer_pubkey = %me, "DPoS peer identity");

        // ── Live per-epoch threshold-beacon DKG actor ───────────────────────────
        // committee[E] deals to itself over BEACON_CHANNEL during E-1; the agreed
        // (PK_E, share) is memoized into `ceremony_store` before E's boundary block
        // and read by the consensus verify/propose path (Phase 4) + the per-epoch
        // signing-slot swap (Phase 5). The store is held here and threaded in below.
        let ceremony_store: crate::beacon::actor::CeremonyStore =
            Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));

        // Committee resolver — shared by the DKG actor (deal/carry-forward
        // decision) and the verify/propose beacon gate. Reads the on-chain
        // committee at the current finalized EVM hash.
        let committee_for: crate::beacon::actor::CommitteeFor = {
            let reader = reader_for_dkg;
            let provider = provider_for_dkg;
            Arc::new(move |epoch: u64| {
                let fin = provider.finalized_block_number().ok().flatten()?;
                let hash = provider.block_hash(fin).ok().flatten()?;
                let snap = reader.epoch_committee_snapshot(epoch, hash).ok()?;
                if snap.validators.is_empty() {
                    return None;
                }
                Some(commonware_utils::ordered::Set::from_iter_dedup(
                    snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
                ))
            })
        };

        // Verify/propose beacon context: the boundary "C" gate reads this node's
        // memoized (PK_E, share) from the live-DKG store; the proposer asserts
        // PK_E in `beacon_outcome`.
        let beacon_for_epoch: crate::application::BeaconForEpoch = {
            let store = ceremony_store.clone();
            Arc::new(move |epoch: u64| store.read().ok().and_then(|m| m.get(&epoch).cloned()))
        };
        let beacon_verify = crate::application::BeaconVerify::new(
            beacon_for_epoch,
            committee_for.clone(),
            dpos_activation_block,
            interval.into(),
        );

        {
            let dkg_namespace = crate::beacon::seed::seed_namespace(&fluent_namespace(chain_id));
            let dkg_actor = crate::beacon::actor::DkgActor::new(
                dkg_namespace,
                peer_keypair.clone(),
                handles.beacon_sender,
                handles.beacon_receiver,
                committee_for,
                ceremony_store.clone(),
                dpos_activation_block,
                interval.into(),
            );
            drop(ctx.with_label("dkg_actor").spawn(move |c| async move {
                dkg_actor.run(dkg_height_rx, c).await;
            }));
        }

        // Isolation-window watchdog: a non-committee `--dpos` node has ZERO
        // consensus-plane connectivity (the tracked peer set == the on-chain
        // committee) yet otherwise looks alive — the silent-verifier trap.
        // Surface it: when finalized makes no progress across two ticks AND
        // this key is not in the current committee, say so loudly. Expected
        // for a not-yet-committee validator without --dpos.follower-upstream;
        // anything else means registration/delegation needs checking.
        {
            let wd_reader = RethStakingStateReader::new(
                provider.clone(),
                evm_config.clone(),
                staking_config.clone(),
            );
            let wd_provider = provider.clone();
            let wd_me = me.clone();
            let wd_interval = interval;
            let wd_activation = dpos_activation_block;
            drop(
                ctx.with_label("committee_watchdog")
                    .spawn(move |c| async move {
                        let mut prev_fin = 0u64;
                        let mut stagnant = 0u32;
                        let mut cached: Option<(u64, B256, Option<bool>)> = None;
                        loop {
                            c.sleep(Duration::from_secs(60)).await;
                            let Ok(Some(fin)) = wd_provider.finalized_block_number() else {
                                continue;
                            };
                            if fin > prev_fin {
                                prev_fin = fin;
                                stagnant = 0;
                                continue;
                            }
                            stagnant += 1;
                            if stagnant < 2 {
                                continue;
                            }
                            let Ok(Some(hash)) = wd_provider.block_hash(fin) else {
                                continue;
                            };
                            let epoch = fluentbase_staking_reader::reader::epoch_of_block(
                                fin,
                                wd_interval,
                                wd_activation,
                            );
                            // The snapshot is a deterministic state read — while
                            // finalized is stagnant its inputs cannot change, so a
                            // wedged verifier re-warns from cache instead of re-running
                            // the committee EVM read every tick for hours.
                            let in_committee = match cached {
                                Some((c_fin, c_hash, verdict))
                                    if (c_fin, c_hash) == (fin, hash) =>
                                {
                                    verdict
                                }
                                _ => {
                                    let verdict = wd_reader
                                        .epoch_committee_snapshot(epoch, hash)
                                        .ok()
                                        .map(|s| {
                                            s.validators.iter().any(|v| v.keys.peer_pubkey == wd_me)
                                        });
                                    cached = Some((fin, hash, verdict));
                                    verdict
                                }
                            };
                            if in_committee == Some(false) {
                                warn!(
                                    finalized = fin,
                                    epoch,
                                    "no finalized progress and this key is NOT in the current \
                             committee — run unified mode (--dpos.follower-upstream) to \
                             follow meanwhile; otherwise check registration/delegation"
                                );
                            }
                        }
                    }),
            );
        }

        // Slasher wiring — `latest_finalized_hash` closure over the reth
        // provider. The TxPool transport sink arrives pre-built via
        // `cfg.slasher_sink` (host-side construction).
        let provider_for_finalized = provider.clone();
        let slasher_latest_finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync> =
            Arc::new(move || {
                let n = provider_for_finalized.finalized_block_number().ok()??;
                provider_for_finalized.block_hash(n).ok().flatten()
            });

        // Per-epoch threshold beacon key for the combined consensus scheme: each
        // vote carries the seed partial (round-keyed), so the seed is recovered
        // from the notarization/finalization certificate — no separate seed
        // plane. Single genesis-bootstrapped key for now (per-epoch DKG is phased).
        let beacon_key = beacon.map(|bl| {
            let namespace = crate::beacon::seed::seed_namespace(&fluent_namespace(chain_id));
            (bl.sharing, bl.share, namespace)
        });

        let outer = OuterBuilder {
            me: me.clone(),
            blocker: handles.oracle.clone(),
            provider: handles.oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block,
            signer_keypair: Some(bls_keypair),
            mode_events,
            beacon_key,
            beacon_verify: Some(beacon_verify),
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
            deque_size: 64,
            partition_prefix: MARSHAL_PARTITION_PREFIX.into(),
            resolver_initial: Duration::from_secs(1),
            resolver_timeout: Duration::from_secs(2),
            resolver_fetch_retry: Duration::from_millis(100),

            // FluentApp constructor args.
            genesis: genesis_block,
            beacon_engine: beacon_engine_handle,
            deriver,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            boundary_hook,

            // Executor cold-start state.
            last_execution_finalized_height,
            initial_finalized: (Height::new(latest_finalized), latest_finalized_hash),
            initial_head: (Height::new(initial_head_num), initial_head_hash),
            // DPoS-era floor: the marshal never dispatches pre-anchor history.
            // Fresh migration: anchor = activation. Promotion: anchor = the
            // EL-restored finalized tip (no pre-anchor certs exist locally).
            // Restart: a raises-only no-op (the archive's floor is already
            // at/above its own finalized).
            marshal_floor: Some(Height::new(latest_finalized)),
            seed_anchor_block: kind == ColdStartKind::Promotion,
            fcu_heartbeat_interval: Duration::from_secs(8),
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_address,
            slasher_reader: reader_for_slasher,
            slasher_latest_finalized_hash,
            slasher_stale_fallback,
            slasher_sink,
            slasher_wal_partition: "slasher-wal".into(),

            feed,
        }
        .build(ctx.with_label("outer_engine"))
        .await?;

        // Register the initial epoch's BlsScheme so marshal can verify
        // certificates from this epoch before any boundary fires.
        let namespace = fluent_namespace(chain_id);
        let initial_committee = epoch_committee_from_snapshot(&initial_snapshot)
            .map_err(|e| eyre!("initial snapshot has non-unique participants: {e:?}"))?;
        let initial_scheme = build_verifier(&namespace, initial_committee.bimap, None);
        outer.cold_start_register(Epoch::new(initial_epoch_u64), initial_scheme);

        // Bridge forwarder: drains (u64, snap) queued by EpochTransition
        // and converts to (Epoch, snap) for OuterEngine's boundary receiver.
        let outer_boundary_tx = outer.boundary_sender();
        let shutdown_for_forwarder = shutdown.clone();
        drop(ctx.with_label("epoch_bridge").spawn(move |_| async move {
            while let Some((u64_ep, snap)) = bridge_rx.recv().await {
                if let Err(e) = outer_boundary_tx.send((Epoch::new(u64_ep), snap)).await {
                    error!(
                        epoch = u64_ep,
                        error = %e,
                        "OuterEngine.boundary_sender receiver dropped — initiating shutdown"
                    );
                    shutdown_for_forwarder.cancel();
                    break;
                }
            }
        }));

        // Grab the marshal mailbox for the node-side cert feed/RPC BEFORE
        // `start` consumes the engine.
        let cert_mailbox = outer.marshal_mailbox();

        // Start OuterEngine — 6-arg start: ctx + 5 raw channels.
        let consensus_handle = outer.start(
            ctx.with_label("marshal_resolver"),
            (handles.vote_sender, handles.vote_receiver),
            (handles.cert_sender, handles.cert_receiver),
            (handles.resolver_sender, handles.resolver_receiver),
            (handles.broadcast_sender, handles.broadcast_receiver),
            (handles.marshal_sender, handles.marshal_receiver),
        );

        Ok(DposLayerHandle {
            consensus_handle,
            network_handle,
            cert_mailbox,
        })
    }
}

#[cfg(test)]
mod cold_start_kind_tests {
    use super::{resolve_cold_start_kind, ColdStartKind};

    const ACTIVATION: u64 = 192;
    const INTERVAL: u32 = 64;

    #[test]
    fn overshoot_with_empty_archive_is_fatal_pointing_at_unified_mode() {
        let err = resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 500).unwrap_err();
        assert!(
            err.to_string().contains("--dpos.follower-upstream"),
            "{err}"
        );
    }

    #[test]
    fn inside_epoch_zero_is_fresh_migration() {
        let kind =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 10).expect("fresh");
        assert_eq!(kind, ColdStartKind::FreshMigration);
    }

    #[test]
    fn archive_present_is_restart() {
        let kind =
            resolve_cold_start_kind(ACTIVATION + 300, ACTIVATION, INTERVAL, ACTIVATION + 500)
                .expect("restart");
        assert_eq!(kind, ColdStartKind::Restart);
    }

    #[test]
    fn boundary_exactly_one_interval_is_overshoot() {
        // cs_finalized == activation + interval is the FIRST fatal height
        // (epoch 0 is [activation, activation + interval)).
        let err = resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + INTERVAL as u64)
            .unwrap_err();
        assert!(err.to_string().contains("past epoch 0"), "{err}");
    }
}

#[cfg(test)]
mod visibility_retry_tests {
    use super::*;
    use crate::{application::ParentHeaderMissing, digest::Digest, order_block::OrderBlock};
    use alloy_consensus::{Block as AlloyBlock, BlockBody};
    use alloy_primitives::{Address, Bytes};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_ethereum_primitives::TransactionSigned;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn sample_order() -> OrderBlock {
        OrderBlock {
            parent: Digest(B256::ZERO),
            height: 7,
            timestamp: 7,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            beacon_outcome: None,
            beacon_seed: None,
        }
    }

    /// Fails the first `transient_failures` calls with [`ParentHeaderMissing`]
    /// (or `fatal` instead, when set), then succeeds.
    struct FlakyDeriver {
        calls: AtomicU32,
        transient_failures: u32,
        fatal: bool,
    }

    impl DerivedBlockBuilder for FlakyDeriver {
        type Derived = SealedBlock<RethBlock>;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
            _seed: Option<crate::beacon::types::Seed>,
        ) -> eyre::Result<SealedBlock<RethBlock>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.transient_failures {
                if self.fatal {
                    return Err(eyre!("disk exploded"));
                }
                return Err(ParentHeaderMissing(parent_evm_hash).into());
            }
            let header = Header {
                parent_hash: parent_evm_hash,
                number: order.height,
                ..Default::default()
            };
            let body: BlockBody<TransactionSigned> = BlockBody::default();
            Ok(SealedBlock::seal_slow(RethBlock::from(AlloyBlock::new(
                header, body,
            ))))
        }
    }

    #[test]
    fn transient_parent_miss_is_retried_until_visible() {
        deterministic::Runner::default().start(|ctx| async move {
            let deriver = FlakyDeriver {
                calls: AtomicU32::new(0),
                transient_failures: 3,
                fatal: false,
            };
            let derived =
                derive_with_visibility_retry(&ctx, &deriver, &sample_order(), B256::ZERO, None)
                    .await
                    .expect("recovers once the parent becomes visible");
            assert_eq!(derived.number(), 7);
            assert_eq!(deriver.calls.load(Ordering::SeqCst), 4);
        });
    }

    #[test]
    fn other_derivation_errors_stay_immediately_fatal() {
        deterministic::Runner::default().start(|ctx| async move {
            let deriver = FlakyDeriver {
                calls: AtomicU32::new(0),
                transient_failures: 1,
                fatal: true,
            };
            let err =
                derive_with_visibility_retry(&ctx, &deriver, &sample_order(), B256::ZERO, None)
                    .await
                    .unwrap_err();
            assert!(err.to_string().contains("disk exploded"), "{err}");
            assert_eq!(
                deriver.calls.load(Ordering::SeqCst),
                1,
                "no retry on fatal errors"
            );
        });
    }

    #[test]
    fn persistent_parent_miss_fails_after_deadline() {
        deterministic::Runner::default().start(|ctx| async move {
            let deriver = FlakyDeriver {
                calls: AtomicU32::new(0),
                transient_failures: u32::MAX,
                fatal: false,
            };
            let err =
                derive_with_visibility_retry(&ctx, &deriver, &sample_order(), B256::ZERO, None)
                    .await
                    .unwrap_err();
            assert!(err.downcast_ref::<ParentHeaderMissing>().is_some(), "{err}");
            assert!(
                deriver.calls.load(Ordering::SeqCst) > 1,
                "must have retried before giving up"
            );
        });
    }
}
