//! DPoS layer launcher — assembles 03 (staking-reader), 04 (consensus),
//! and 05 (p2p) given operator keys, reth handles, and config. Spawned
//! by the host adapter at `crates/node/src/dpos.rs`.

use crate::{
    application::{BeaconEngineLike, PayloadAttrsBuilderLike, PayloadBuilderLike},
    block::Block as ConsensusBlock,
    scheme::epoch_committee_from_snapshot,
    slasher::actor::{SharedCacheFallback, SlasherTxSink, StaleEpochFallback},
    timeouts::ConsensusTimeouts,
    OuterBuilder,
};
use alloy_consensus::Header;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::{ForkchoiceState, PayloadId};
use commonware_consensus::{
    types::{Epoch, Height},
    Heightable as _,
};
use commonware_cryptography::Signer;
use commonware_p2p::{authenticated::discovery::Bootstrapper, Ingress};
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use commonware_storage::{
    archive::{Archive as _, Identifier},
    metadata::{self, Metadata},
};
use commonware_utils::sequence::U64;
use dashmap::DashMap;
use eyre::{ensure, eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::{
    fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_verifier, PeerPubkey,
};
use fluentbase_p2p::{FluentP2P, FluentP2PConfig};
use fluentbase_staking_reader::{
    reader::StakingReaderConfig, EpochTransition, ReadError, RethStakingStateReader,
    TransitionOutcome, ValidatorSetCache,
};
use reth_ethereum_engine_primitives::EthPayloadAttributes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
use reth_primitives_traits::SealedBlock;
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::{
    marker::PhantomData,
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
use tracing::{error, info};

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
pub struct RethHandle<Provider, EvmConfig, PayloadBuilder, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub payload_builder_handle: PayloadBuilder,
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
                 wait for the sequencer to finalize it before starting DPoS (ensure graceful \
                 shutdown so blocks persist to MDBX)"
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
async fn recover_finalized_tail_into_reth<Provider, BeaconEngine>(
    ctx: &Context,
    beacon_engine: &BeaconEngine,
    provider: &Provider,
    target: u64,
) -> eyre::Result<B256>
where
    Provider: BlockHashReader,
    BeaconEngine: BeaconEngineLike<
        PayloadAttrs = EthPayloadAttributes,
        ExecutionData = SealedBlock<RethBlock>,
    >,
{
    // An ungraceful crash loses only reth's unflushed tail (typically 1-2 blocks).
    // A larger gap is real EL data loss, not a recoverable flush race.
    const MAX_COLD_RECOVER: u64 = 64;

    let archive = crate::outer::init_finalized_blocks_archive(ctx, MARSHAL_PARTITION_PREFIX).await;

    // Collect blocks reth is missing, from `target` downward, until reth holds the
    // parent (the reconnect point).
    let mut to_apply: Vec<ConsensusBlock> = Vec::new();
    let mut h = target;
    while provider
        .block_hash(h)
        .wrap_err("provider.block_hash during crash-survivor recovery")?
        .is_none()
    {
        if to_apply.len() as u64 >= MAX_COLD_RECOVER {
            return Err(eyre!(
                "crash-survivor recovery: reth missing > {MAX_COLD_RECOVER} blocks below \
                 finalized {target} — real EL data loss; re-sync the EL disk from a snapshot"
            ));
        }
        let block = archive
            .get(Identifier::Index(h))
            .await
            .map_err(|e| eyre!("reading marshal finalized_blocks at height {h}: {e}"))?
            .ok_or_else(|| {
                eyre!(
                    "crash-survivor recovery: marshal finalized_blocks has no block at height \
                     {h} (gap exceeds marshal retention); re-sync the EL disk from a snapshot"
                )
            })?;
        to_apply.push(block);
        if h == 0 {
            break;
        }
        h -= 1;
    }

    // The target block was pushed first; capture its EL hash for the canonicalizing
    // FCU below before the vec is consumed.
    let target_hash = to_apply
        .first()
        .expect("recovery loop ran at least once (target was missing)")
        .block_hash();

    // Apply oldest-first so each block's parent is already present in reth.
    for block in to_apply.into_iter().rev() {
        let bh = block.height();
        let status = beacon_engine
            .new_payload(block.into_inner())
            .await
            .wrap_err("crash-survivor recovery new_payload failed")?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL rejected recovered finalized block {bh}: {status:?}"
        );
    }
    drop(archive); // release so MarshalActor::init can re-open the same partition.

    // `new_payload` only buffers/validates; an FCU is required to make the recovered
    // tail canonical so `block_hash(target)` (and the committee/genesis reads that
    // follow) actually see it.
    let resp = beacon_engine
        .fork_choice_updated(
            ForkchoiceState {
                head_block_hash: target_hash,
                safe_block_hash: target_hash,
                finalized_block_hash: target_hash,
            },
            None,
        )
        .await
        .wrap_err("crash-survivor recovery FCU failed")?;
    ensure!(
        resp.is_valid(),
        "EL rejected crash-survivor recovery FCU (target={target}): {:?}",
        resp.payload_status
    );

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
pub struct DposLayerConfig<AB> {
    pub bls_keypair: ValidatorBlsKeypair,
    pub peer_keypair: commonware_cryptography::ed25519::PrivateKey,
    pub slasher_sink: Arc<dyn SlasherTxSink>,
    pub staking_config: StakingReaderConfig,
    pub bootstrappers: Vec<Bootstrapper<PeerPubkey>>,
    pub p2p: P2pParams,
    pub payload_attrs_builder: AB,
    pub extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
    /// Cert-feed sink (node-built): wired as the marshal's second
    /// application-`Reporter` so a node-side feed actor can serve the
    /// `consensus` RPC. `None` for nodes that don't serve the cert feed.
    pub feed: Option<crate::feed_sink::FeedSink>,
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
    pub async fn launch<Provider, EvmConfig, PayloadBuilder, BeaconEngine, AB>(
        ctx: Context,
        reth: RethHandle<Provider, EvmConfig, PayloadBuilder, BeaconEngine>,
        cfg: DposLayerConfig<AB>,
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
        PayloadBuilder: PayloadBuilderLike<BuiltSealed = SealedBlock<RethBlock>>
            + Clone
            + Send
            + Sync
            + 'static,
        BeaconEngine: BeaconEngineLike<
                PayloadAttrs = EthPayloadAttributes,
                ExecutionData = SealedBlock<RethBlock>,
            > + Clone
            + Send
            + Sync
            + 'static,
        AB: PayloadAttrsBuilderLike<Attrs = EthPayloadAttributes, Header = Header>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let DposLayerConfig {
            bls_keypair,
            peer_keypair,
            slasher_sink,
            staking_config,
            bootstrappers,
            p2p: P2pParams { listen, dialable },
            payload_attrs_builder,
            extra_data_registry,
            feed,
        } = cfg;

        let RethHandle {
            provider,
            evm_config,
            payload_builder_handle,
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
        let reader_for_slasher =
            RethStakingStateReader::new(provider.clone(), evm_config, staking_config);

        // Reth's `BlockchainProvider::with_latest` populates
        // `canonical_state.finalized_block` from
        // `ChainState::LastFinalizedBlock` during node init, so on a
        // graceful-shutdown restart `get_finalized_num_hash()` returns
        // `Some(disk_finalized.num_hash())`. The genesis fallback
        // handles the pristine-network case (no FCU yet).
        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head block number at startup")?;
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

        // Two-way cold-start discriminator (restart vs fresh migration). The
        // marshal's durable application-metadata is the signal: an empty store
        // (height <= activation) is a fresh Tempo→DPoS migration; a populated
        // store (> activation) is a restart of an already-migrated node, which
        // MUST resume at its real finalized height so the scheme cascade starts
        // at the correct epoch.
        let archive_finalized =
            read_consensus_archive_last_finalized(&ctx, MARSHAL_PARTITION_PREFIX).await?;
        let is_fresh_migration = archive_finalized <= dpos_activation_block;
        let (latest_finalized, latest_finalized_hash) = if is_fresh_migration {
            // FRESH MIGRATION: anchor ≡ block@dposActivationBlock; wait for reth
            // to hold it, hash derived locally (canonical at a finalized height).
            // Checkpoint-provisioned EL-ahead start is deferred to Phase 2/β — a
            // node-local EL head as genesis would diverge cross-node.
            let hash = wait_for_activation_block(&ctx, &provider, dpos_activation_block).await?;
            // fail-loud: the sequencer must not have overshot epoch 0 entirely
            // (finalized into epoch 1+) — DPoS would otherwise sibling-fork
            // already-finalized blocks → finalized rollback → split.
            ensure!(
                cs_finalized < dpos_activation_block + interval as u64,
                "sequencer overshot epoch 0 (finalized {cs_finalized} >= activation \
                 {dpos_activation_block} + interval {interval}); refusing to anchor DPoS"
            );
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
                        archive_finalized,
                    )
                    .await?
                }
            };
            (archive_finalized, hash)
        };

        tracing::info!(
            last_execution_finalized_height,
            archive_finalized,
            is_fresh_migration,
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
        if is_fresh_migration {
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
        //   `ChainConfig.activeValidatorsLength <= fluentbase_p2p::MAX_PEER_SET_SIZE`.
        let active_validators_length = reader
            .active_validators_length(latest_finalized_hash)
            .wrap_err("failed reading ChainConfig.activeValidatorsLength")?;
        if (active_validators_length as u64) > fluentbase_p2p::constants::MAX_PEER_SET_SIZE {
            return Err(eyre!(
                "ChainConfig.activeValidatorsLength ({}) exceeds \
                 fluentbase_p2p::constants::MAX_PEER_SET_SIZE ({}). Rust ↔ Solidity \
                 cap drift detected — bump MAX_PEER_SET_SIZE in \
                 crates/p2p/src/constants.rs AND MAX_ACTIVE_VALIDATORS in \
                 solidity-contracts/contracts/staking/ChainConfig.sol in the SAME PR, \
                 then redeploy/upgrade.",
                active_validators_length,
                fluentbase_p2p::constants::MAX_PEER_SET_SIZE,
            ));
        }

        info!(
            chain_id,
            interval,
            retention,
            max_peer_set_size = fluentbase_p2p::constants::MAX_PEER_SET_SIZE,
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
        let mut epoch_transition = EpochTransition::new(
            reader,
            cache,
            handles.oracle.clone(),
            fluentbase_p2p::constants::MAX_PEER_SET_SIZE as usize,
            Some(bridge_tx.clone()),
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
        let genesis_block = ConsensusBlock::from_execution_block(genesis_sealed);

        // Move EpochTransition into Arc<Mutex<_>> so the boundary_hook
        // closure can call back into it from any thread.
        let et_arc = Arc::new(Mutex::new(epoch_transition));

        // Boundary hook: fires for every `Update::Block`. Spawns
        // fire-and-forget via `ctx.spawn` (NOT `tokio::spawn`, which would
        // depend on the implicit `tokio::Handle::current()` contract under
        // commonware-tokio).
        let consecutive_errors = Arc::new(AtomicU32::new(0));
        let shutdown_for_hook = shutdown.clone();
        let et_for_hook = et_arc.clone();
        let ctx_for_hook = ctx.with_label("boundary_hook");
        let errors_for_hook = consecutive_errors.clone();
        let boundary_hook: Arc<dyn Fn(ConsensusBlock) + Send + Sync> =
            Arc::new(move |block: ConsensusBlock| {
                let et = et_for_hook.clone();
                let ctx_task = ctx_for_hook.clone();
                let shutdown = shutdown_for_hook.clone();
                let errors = errors_for_hook.clone();
                let hash = block.block_hash();
                let number = block.height().get();
                drop(ctx_task.spawn(move |ctx_inner| async move {
                    // `BlockNotFound` is the executor/new_payload race: this hook
                    // and the executor both consume the same `Update::Block`, and
                    // the hook's reth state read can beat the executor's
                    // `new_payload` by microseconds. Intra-epoch this is benign
                    // and self-corrects on the next finalized block — BUT the LAST
                    // block of an epoch has no "next" finalized block until
                    // `on_finalized` spawns the next epoch's engine, so a boundary
                    // block that loses this race deferred-to-next would wedge the
                    // chain forever (the epoch E+1 engine that produces block
                    // E_end+1 is exactly what the deferred spawn was meant to start).
                    // So retry in place — the block lands within a few ms — instead
                    // of deferring to a finalized block that can never arrive.
                    const BLOCK_LANDED_RETRIES: u32 = 100; // ~5s @ 50ms backoff
                    const BLOCK_LANDED_BACKOFF: Duration = Duration::from_millis(50);
                    let mut race_retries = 0u32;
                    loop {
                        // Re-lock per attempt so the retry backoff never holds the
                        // EpochTransition mutex across a sleep. on_finalized is
                        // idempotent, so re-entry after BlockNotFound is safe.
                        let outcome = {
                            let mut et_guard = et.lock().await;
                            et_guard.on_finalized(hash, number).await
                        };
                        match outcome {
                            Ok(TransitionOutcome::EpochAdvanced(_)) => {
                                // Reset only on a real epoch advance.
                                errors.store(0, Ordering::Relaxed);
                                break;
                            }
                            Ok(TransitionOutcome::Intra) => {
                                // Intra-epoch re-delivery, missed-commit epoch, or
                                // bridge-channel-full retry-stall.
                                break;
                            }
                            Err(ReadError::BlockNotFound(_))
                                if race_retries < BLOCK_LANDED_RETRIES =>
                            {
                                race_retries += 1;
                                ctx_inner.sleep(BLOCK_LANDED_BACKOFF).await;
                                continue;
                            }
                            // Block never landed after the full backoff window —
                            // this is no longer a microsecond race; treat as a real
                            // error so the consecutive-error threshold can act.
                            Err(ReadError::BlockNotFound(_)) => {
                                let count = errors.fetch_add(1, Ordering::Relaxed) + 1;
                                error!(
                                    block_number = number,
                                    block_hash = ?hash,
                                    consecutive_errors = count,
                                    retries = race_retries,
                                    "epoch_transition.on_finalized: finalized block never \
                                     landed in reth after retry window; treating as fatal"
                                );
                                if count >= MAX_CONSECUTIVE_ON_FINALIZED_ERRORS {
                                    shutdown.cancel();
                                }
                                break;
                            }
                            Err(e) => {
                                let count = errors.fetch_add(1, Ordering::Relaxed) + 1;
                                error!(
                                    block_number = number,
                                    block_hash = ?hash,
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
                    }
                }));
            });

        let me = peer_keypair.public_key();
        info!(peer_pubkey = %me, "DPoS peer identity");

        // Slasher wiring — `latest_finalized_hash` closure over the reth
        // provider. The TxPool transport sink arrives pre-built via
        // `cfg.slasher_sink` (host-side construction).
        let provider_for_finalized = provider.clone();
        let slasher_latest_finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync> =
            Arc::new(move || {
                let n = provider_for_finalized.finalized_block_number().ok()??;
                provider_for_finalized.block_hash(n).ok().flatten()
            });

        let outer = OuterBuilder {
            me: me.clone(),
            blocker: handles.oracle.clone(),
            provider: handles.oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block,
            signer_keypair: Some(bls_keypair),
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
            deque_size: 64,
            partition_prefix: MARSHAL_PARTITION_PREFIX.into(),
            resolver_initial: Duration::from_secs(1),
            resolver_timeout: Duration::from_secs(2),
            resolver_fetch_retry: Duration::from_millis(100),

            // FluentApp constructor args.
            genesis: genesis_block,
            payload_builder: payload_builder_handle,
            beacon_engine: beacon_engine_handle,
            payload_attrs_builder,
            payload_resolve_time: Duration::from_millis(300),
            boundary_hook,

            // Executor cold-start state.
            last_execution_finalized_height,
            initial_finalized: (Height::new(latest_finalized), latest_finalized_hash),
            initial_head: (Height::new(initial_head_num), initial_head_hash),
            // DPoS-era floor: the marshal never dispatches pre-activation history
            // (no DPoS node holds it). Seeds a fresh migration to anchor+1; a
            // no-op raise on restart (the archive's floor is already higher).
            marshal_floor: Some(Height::new(dpos_activation_block)),
            fcu_heartbeat_interval: Duration::from_secs(8),
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            _attrs: PhantomData::<EthPayloadAttributes>,

            slasher_staking_address: staking_address,
            slasher_reader: reader_for_slasher,
            slasher_latest_finalized_hash,
            slasher_stale_fallback,
            slasher_sink,
            slasher_wal_partition: "slasher-wal".into(),

            extra_data_registry: extra_data_registry.clone(),
            feed,
        }
        .build(ctx.with_label("outer_engine"))
        .await?;

        // Register the initial epoch's BlsScheme so marshal can verify
        // certificates from this epoch before any boundary fires.
        let namespace = fluent_namespace(chain_id);
        let initial_committee = epoch_committee_from_snapshot(&initial_snapshot)
            .map_err(|e| eyre!("initial snapshot has non-unique participants: {e:?}"))?;
        let initial_scheme = build_verifier(&namespace, initial_committee.bimap);
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
