//! DPoS layer launcher — assembles 03 (staking-reader), 04 (consensus),
//! and 05 (p2p) given operator keys, reth handles, and config. Spawned
//! by the host adapter at `crates/node/src/dpos.rs`.

use crate::{
    application::{
        derive_with_visibility_retry, BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder,
        ExecutedChain, OrderingAssembler,
    },
    cold_start_jump::ElSync as _,
    executed::executed_state_hash,
    order_block::{anchor_order_block, OrderBlock, K},
    scheme::epoch_committee_from_snapshot,
    slasher::actor::SlasherTxSink,
    sync_metrics::{SyncMetrics, SyncReason},
    timeouts::ConsensusTimeouts,
    OuterBuilder, SoftEnterCommittees,
};
use alloy_consensus::Header;
use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::types::{Epoch, Height};
use commonware_cryptography::Signer;
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
use fluentbase_p2p::NoopBlocker;
use fluentbase_staking_reader::{
    reader::{StakingReaderConfig, ValidatorSetSnapshot},
    EpochTransition, RethStakingStateReader, TransitionOutcome,
};
use prometheus_client::metrics::{counter::Counter, family::Family, gauge::Gauge};
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
use reth_primitives_traits::SealedBlock;
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Codeless-tolerant epoch-geometry read: `None` when `ChainConfig` is not
/// deployed (or DPoS not yet scheduled) at `at` — the launch discriminator
/// between "restart datadir / genesis-baked devnet" and "fresh datadir on a
/// runtime-deployed chain", where geometry is only readable AFTER EL-sync. Used
/// by the follower cold-start in [`DposLayer::launch_follower`].
fn read_geometry<Provider, EvmConfig>(
    reader: &RethStakingStateReader<Provider, EvmConfig>,
    at: B256,
) -> eyre::Result<Option<(u64, u32)>>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    match reader.scheduled_dpos_activation(at)? {
        None => Ok(None),
        Some(activation) => {
            let interval = reader.epoch_block_interval(at)?;
            ensure!(interval > 0, "epoch_block_interval must be > 0");
            Ok(Some((activation, interval)))
        }
    }
}

/// Re-poke count between successive "boundary still parked" `warn!`s (log-side
/// external wedge detection alongside the `parked_boundary_height` gauge). At
/// `PENDING_RETRY_BACKOFF = 200 ms`, 150 pokes ≈ 30 s.
const PARKED_BOUNDARY_WARN_EVERY: u64 = 150;

/// Partition prefix for the commonware marshal's durable storage (finalizations,
/// finalized blocks, application-metadata). Shared between the cold-start
/// discriminator peek (`read_consensus_archive_last_finalized`) and the marshal
/// itself (`OuterBuilder.partition_prefix`) so the two never drift.
const MARSHAL_PARTITION_PREFIX: &str = "consensus_marshal";

/// Partition for the durable `Round → σ` store behind the
/// [`SeedStore`](crate::beacon::certify::SeedStore). Deliberately NOT under
/// [`MARSHAL_PARTITION_PREFIX`]: this is a Fluent-side store beside the marshal's,
/// not part of it, and it must stay independently prunable.
///
/// Renamed from `beacon-seed-journal` when the backing primitive moved from
/// `journal::segmented::fixed` to `ordinal::Ordinal`: the two on-disk formats are
/// incompatible, and pointing at a fresh name lets the retention window simply
/// refill. That is free ONLY while the data is miss-if-lost — `parent_seed` still
/// couriers σ in the child block, so a cold seed store costs nothing but a
/// re-derive. **The next format change after `parent_seed` is removed is NOT
/// free** and will need a real migration.
pub(crate) const SEED_JOURNAL_PARTITION: &str = "beacon-seed-ordinal";

/// Partition of the durable `epoch → PK_epoch` store. Empty would mean RAM-only.
pub(crate) const KEY_JOURNAL_PARTITION: &str = "beacon-key-ordinal";

/// Partition of the durable `epoch → agreement artifact` store
/// ([`crate::beacon::artifact::ArtifactStore`]). Public because the store is
/// opened by the always-on beacon plane in the node crate, one process-wide
/// instance — a second handle over this partition would be a dual-writer.
pub const ARTIFACT_JOURNAL_PARTITION: &str = "beacon-artifact-metadata";

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
    /// Read-only probe of reth's connected devp2p peer count, built host-side
    /// from `node.network` (which implements `reth_network_api::PeersInfo`). Drives
    /// the cold-start / re-jump EL-sync no-peers net (`cold_start_jump::RethElSync`).
    /// A closure, not a typed handle, keeps the consensus crate free of a
    /// `reth-network-api` dependency.
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
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

/// Wait for reth to hold the DPoS activation block before adopting it as the
/// fresh-migration consensus anchor; returns the block's local-canonical hash.
/// Covers reth still replaying MDBX on restart. There is NO give-up and no
/// timeout: the anchor is external (the sequencer must finalize the activation
/// block before DPoS starts), so the wait polls forever and raises
/// `dpos_sync_degraded{reason=activation_wait}` as the stuck signal instead of
/// failing. No operator-hash compare — the activation height comes from the on-chain
/// `ChainConfig.dposActivationBlock` and the hash is local-canonical at a
/// finalized height (every honest node derives the same hash).
pub(crate) async fn wait_for_activation_block<Provider, C>(
    ctx: &C,
    provider: &Provider,
    activation: u64,
    sync_metrics: &SyncMetrics,
) -> eyre::Result<B256>
where
    Provider: BlockHashReader,
    C: commonware_runtime::Clock,
{
    // #13 SELF-HEAL (retry-forever, Decision A): the activation anchor is EXTERNAL
    // — the sequencer may still be producing/persisting it, and a fatal here would
    // restart-storm every honest joiner at once. Keep polling forever and surface
    // `dpos_sync_degraded{reason=activation_wait}=1` as the stuck signal (a stuck
    // gauge, not a crash). A cold restart legitimately shows the block absent for
    // seconds while reth replays MDBX/static files under multi-node contention.
    const POLL: Duration = Duration::from_secs(2);
    let mut waited = false;
    loop {
        if let Some(hash) = provider
            .block_hash(activation)
            .wrap_err("provider.block_hash failed during activation-block wait")?
        {
            if waited {
                sync_metrics.recover(SyncReason::ActivationWait);
            }
            tracing::info!(height = activation, hash = ?hash, "DPoS activation block present in reth");
            return Ok(hash);
        }
        if !waited {
            waited = true;
            warn!(
                height = activation,
                "reth does not yet hold the DPoS activation block; polling (no give-up) — \
                 waiting for the sequencer to produce and persist it"
            );
        }
        sync_metrics.degrade(SyncReason::ActivationWait);
        ctx.sleep(POLL).await;
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

/// Outcome of the pre-engine crash-survivor recovery (#8/#12 self-heal,
/// 2026-07-09).
#[derive(Debug)]
enum RecoverOutcome {
    /// reth now holds `target`; carries its local-canonical hash. The pre-engine
    /// marshal→reth replay bridged the gap — either straight from the local
    /// `finalized_blocks` archive (BLOCKS-ONLY: the seed for each height is the
    /// child block's `parent_seed` witness, never a per-height cert), or, on a
    /// below-floor BLOCK hole (#8), by a BLS-verified by-height re-fetch through
    /// the cert upstream ([`refetch_verified_archive_hole`]) spliced into the
    /// same replay.
    Recovered(B256),
    /// reth is `> MAX_COLD_RECOVER` behind its OWN (INTACT) consensus archive (#12):
    /// the pre-engine replay is capped, so the caller anchors the cold-start at
    /// `provider.best_block_number()` and the eligible `cold_start_jump` devp2p-
    /// backfills the EL. This is a distance problem, NOT an archive hole — the
    /// marshal holds every block, so reth heals from devp2p alone. Needs an upstream
    /// (a no-upstream node is FATAL at the recovery site,
    /// [`crash_recover_defer_or_fatal`]). `gap` = blocks reth is behind `target`
    /// (`crash_recover_gap_blocks` gauge).
    ///
    /// NOTE (why #8 does NOT land here): a #8 hole is BELOW the marshal's finalized
    /// floor (`target == last_processed_height`, the same `LATEST_KEY` the metadata
    /// peek reads). The live marshal repairs only `[floor+1 ..]` (monorepo
    /// `marshal/core/actor.rs:1557` `start = last_processed_height.next()`; it PRUNES
    /// below floor at `:700`, and `HintFinalized` skips `<= floor` at `:633`), so its
    /// `UpstreamResolver` would NEVER repopulate a below-floor hole — and the executor
    /// gap-walk agrees (`executor.rs` "a hole below the floor cannot self-heal"). Nor
    /// does the deferred jump help: a #8 gap is `<= MAX_COLD_RECOVER (64) <
    /// JUMP_THRESHOLD (1024)`, so the jump is always `Lagging` and never fires. #8 is
    /// therefore healed INLINE by [`refetch_verified_archive_hole`], not deferred.
    DeferToElSync { gap: u64 },
}

/// Provider-only reconnect scan for [`recover_finalized_tail_into_reth`] (factored
/// out so the #12 too-deep detection is unit-testable without a marshal archive).
/// Walks `target` downward while reth is missing each parent.
enum ReconnectScan {
    /// reth holds the block at `lowest - 1` (or `lowest == 0`); replay
    /// `lowest..=target` from the marshal archive.
    Reconnect(u64),
    /// reth is missing `>= max_cold_recover` blocks below `target` — beyond a
    /// flush-race tail; #12 defers to devp2p EL-sync.
    TooDeep,
}

fn recover_reconnect_point<Provider>(
    provider: &Provider,
    target: u64,
    max_cold_recover: u64,
) -> eyre::Result<ReconnectScan>
where
    Provider: BlockHashReader,
{
    let mut lowest = target;
    while lowest > 0
        && provider
            .block_hash(lowest - 1)
            .wrap_err("provider.block_hash during crash-survivor recovery")?
            .is_none()
    {
        if target - lowest >= max_cold_recover {
            return Ok(ReconnectScan::TooDeep);
        }
        lowest -= 1;
    }
    Ok(ReconnectScan::Reconnect(lowest))
}

/// The #12 defer-vs-fatal decision for the too-deep trigger (reth `>
/// MAX_COLD_RECOVER` behind its OWN INTACT archive). WITH an upstream: raise the
/// crash-recover gauges + counter and DEFER (the caller anchors at reth's tip; the
/// post-engine devp2p jump backfills the EL — the marshal holds every block, so no
/// consensus-store repair is needed). WITHOUT an upstream: residual FATAL — there is
/// nowhere to devp2p-backfill from, so this is real local data loss (Decision A does
/// not apply to idiosyncratic local corruption). A #8 archive HOLE does NOT route
/// here — it heals inline via [`refetch_verified_archive_hole`].
fn crash_recover_defer_or_fatal<Provider>(
    provider: &Provider,
    target: u64,
    has_upstream: bool,
    sync_metrics: &SyncMetrics,
    cause: &str,
) -> eyre::Result<RecoverOutcome>
where
    Provider: BlockNumReader,
{
    if !has_upstream {
        return Err(eyre!(
            "crash-survivor recovery: {cause}, and no --dpos.follower-upstream is configured to \
             devp2p / by-height re-fetch the gap — real EL/consensus data loss; re-sync the EL \
             disk from a snapshot"
        ));
    }
    let best = provider
        .best_block_number()
        .wrap_err("best_block_number at crash-survivor defer")?;
    let gap = target.saturating_sub(best);
    warn!(
        finalized_target = target,
        reth_best = best,
        gap,
        "crash-survivor recovery: {cause}; deferring to the post-engine devp2p EL-sync \
         (anchor moves to reth's tip; no give-up)"
    );
    sync_metrics.degrade(SyncReason::CrashRecover);
    sync_metrics.crash_recover_deferred_to_elsync.inc();
    sync_metrics.crash_recover_gap_blocks.set(gap as i64);
    Ok(RecoverOutcome::DeferToElSync { gap })
}

/// #8 below-floor archive-hole heal: BLS-verified by-height re-fetch of a missing
/// `finalized_blocks` / `finalizations` entry through the cert upstream.
///
/// A #8 hole sits BELOW the marshal's finalized floor (`target ==
/// last_processed_height`), which the live marshal's own resolver NEVER re-fetches
/// — it repairs `[floor+1 ..]` only and prunes below floor (monorepo
/// `marshal/core/actor.rs:1557`/`:700`/`:633`), and the deferred `cold_start_jump`
/// can't fire either (a #8 gap `<= MAX_COLD_RECOVER (64) < JUMP_THRESHOLD (1024)`
/// ⇒ always `Lagging`). So a bare defer would leave reth permanently missing the
/// block. Instead we pull the finalization+block from the upstream (the SAME
/// by-height seam the inlet uses) and authenticate it EXACTLY like the cold-start
/// jump landing: `verify_jump_structural` (payload == digest) + `verify_jump_
/// authenticated` (2f+1 BLS multisig against `committee[E]` read at `at_hash`, the
/// already-recovered parent's materialized state). The caller then derives + imports
/// the verified block into reth, splicing the hole shut in the same replay.
///
/// We do NOT write the re-fetched entry back into the marshal archive, and the
/// reason is NOT that a below-floor write would be discarded — it would not be.
/// `MarshalActor::init` runs no prune, immutable-archive `prune` is a no-op, and a
/// below-floor entry written before the floor rises stays permanently readable
/// (`get_finalized_block` consults no floor); the boundary-seeding path in
/// `outer.rs` / `executor::reseed_forward` depends on exactly that. The real reason
/// is scope: this path runs PRE-engine against a standalone archive handle, and reth
/// is the only local reader that needs the block at that moment. Peer-serving of
/// that below-floor height stays a re-fetch-from-elsewhere concern, unchanged.
///
/// `upstream == None` (no `--dpos.follower-upstream`) OR the upstream no longer
/// serves the height (gone everywhere) ⇒ residual FATAL — real local consensus
/// data loss with nowhere to re-fetch from.
async fn refetch_verified_archive_hole<U, C>(
    upstream: Option<&U>,
    committees: &C,
    verify_ctx: &mut (impl commonware_runtime::Clock + rand_core::CryptoRngCore),
    at_hash: B256,
    l1_checkpoint: Option<B256>,
    height: u64,
    which: &str,
) -> eyre::Result<crate::cert_follow::UpstreamFinalized>
where
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    let Some(up) = upstream else {
        return Err(eyre!(
            "crash-survivor recovery: marshal {which} has a below-floor hole at height {height}, \
             and no --dpos.follower-upstream is configured to by-height re-fetch the BLS-verified \
             finalization (the marshal's own resolver cannot repair below its finalized floor) — \
             real local consensus data loss; re-sync the EL disk from a snapshot"
        ));
    };
    // `_everywhere`: this arm exits FATAL and tells the operator to re-sync the EL
    // disk from a snapshot. Asking ONE upstream before saying that is not enough
    // when the operator configured several and the block sits on the second.
    let Some(uf) = up.get_finalization_everywhere(Height::new(height)).await else {
        return Err(eyre!(
            "crash-survivor recovery: marshal {which} has a below-floor hole at height {height} \
             and the upstream no longer serves it — the consensus record is gone everywhere; \
             re-sync the EL disk from a snapshot"
        ));
    };
    // Nothing else binds the response to the request: `verify_jump_structural` ties
    // the cert only to the block it arrived with, and `verify_jump_authenticated`
    // takes the epoch from the cert's own round. Unpinned, a valid finalization for
    // a DIFFERENT height passes both and is spliced in as if it were this one.
    ensure!(
        uf.block.height == height,
        "upstream served height {} for the marshal {which} hole at height {height}",
        uf.block.height
    );
    crate::cold_start_jump::verify_jump_structural(&uf).wrap_err_with(|| {
        format!(
            "re-fetched finalization for the marshal {which} hole at height {height} is malformed"
        )
    })?;
    crate::cold_start_jump::verify_jump_authenticated(
        &uf,
        committees,
        at_hash,
        l1_checkpoint,
        verify_ctx,
    )
    .wrap_err_with(|| {
        format!(
            "BLS-authenticating the re-fetched finalization for the marshal {which} hole at \
             height {height} against committee[E] read at the recovered parent {at_hash:?}"
        )
    })?;
    Ok(uf)
}

/// One element of the crash-survivor replay walk: the block at `h` from the
/// marshal's OWN `finalized_blocks` archive, or — on a below-floor BLOCK hole
/// (#8) — a BLS-verified by-height re-fetch through the cert upstream. Under
/// Design B′ the walk is BLOCKS-ONLY: a locally-present block with a locally
/// absent finalization cert is a NORMAL state (an ancestry-finalized height may
/// have NO standalone cert anywhere, ever — the soak7 class), so certs are read
/// solely to AUTHENTICATE a re-fetched missing block, never per height.
#[allow(clippy::too_many_arguments)] // mirrors its caller: distinct pre-engine deps, not a cluster
async fn recover_walk_block<A, U, C>(
    archive: &A,
    upstream: Option<&U>,
    committees: &C,
    verify_ctx: &mut Context,
    at_hash: B256,
    l1_checkpoint: Option<B256>,
    sync_metrics: &SyncMetrics,
    h: u64,
) -> eyre::Result<OrderBlock>
where
    A: commonware_storage::archive::Archive<Value = OrderBlock>,
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    let local = archive
        .get(Identifier::Index(h))
        .await
        .map_err(|e| eyre!("reading marshal finalized_blocks at height {h}: {e}"))?;
    if let Some(order) = local {
        return Ok(order);
    }
    // #8: a hole in the marshal's OWN below-floor BLOCK archive. The live
    // marshal resolver cannot repair below its floor and the deferred jump
    // can't fire at this `<= 64` gap, so a bare defer would strand reth.
    // Re-fetch the BLS-verified finalization+block from the cert upstream and
    // splice the hole shut; no-upstream / gone-everywhere stays fatal.
    let uf = refetch_verified_archive_hole(
        upstream,
        committees,
        verify_ctx,
        at_hash,
        l1_checkpoint,
        h,
        "finalized_blocks",
    )
    .await?;
    sync_metrics.crash_recover_refetched.inc();
    warn!(
        height = h,
        "crash-survivor recovery: healed a below-floor marshal block-archive hole by a \
         BLS-verified by-height re-fetch through the cert upstream"
    );
    Ok(uf.block)
}

/// Crash-survivor cold-start recovery: reth is missing the
/// consensus-finalized block at `target` (an ungraceful crash lost reth's
/// unflushed tail while the marshal persisted the finalization). Read the missing
/// block(s) from the marshal's own `finalized_blocks` archive and `new_payload`
/// them into reth, walking ancestors oldest-ward until reth reconnects; return the
/// recovered `target`'s local hash. Standalone archive open (before the engine is
/// built), like the metadata peek — dropped before `MarshalActor::init` re-opens it.
///
/// A gap wider than `MAX_COLD_RECOVER` (#12 — reth
/// deeply behind its INTACT archive) returns [`RecoverOutcome::DeferToElSync`] (WITH
/// an upstream) for the post-engine devp2p jump. A below-floor HOLE in the marshal's
/// own archive (#8) is instead healed INLINE by a BLS-verified by-height re-fetch
/// through `upstream` ([`refetch_verified_archive_hole`]) — the live marshal resolver
/// cannot repair below its floor and the deferred jump can't fire at a `<= 64` gap, so
/// deferring would strand reth. Both no-upstream cases stay fatal.
// A single-call pre-engine assembly step: each arg is a distinct reth/consensus
// dependency (engine, provider, deriver, upstream, committee source, checkpoint),
// not a bundleable cluster — an args struct would only add indirection.
#[allow(clippy::too_many_arguments)]
async fn recover_finalized_tail_into_reth<Provider, BeaconEngine, D, U, C>(
    ctx: &Context,
    beacon_engine: &BeaconEngine,
    provider: &Provider,
    deriver: &D,
    target: u64,
    upstream: Option<&U>,
    committees: &C,
    l1_checkpoint: Option<B256>,
    sync_metrics: &SyncMetrics,
) -> eyre::Result<RecoverOutcome>
where
    Provider: BlockHashReader + BlockNumReader,
    BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived>,
    D: DerivedBlockBuilder,
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    // The #8 re-fetch authenticates the upstream cert exactly like the cold-start
    // jump landing; `verify_jump_authenticated` needs a `&mut Clock + CryptoRngCore`.
    let mut verify_ctx = ctx.clone();
    // An ungraceful crash loses only reth's unflushed tail (typically 1-2 blocks).
    // A larger gap is NOT a recoverable flush race — #12 defers to devp2p EL-sync.
    const MAX_COLD_RECOVER: u64 = 64;

    // Phase 1 (provider-only): find the reconnect point. A gap wider than the
    // flush-race cap is #12 — the pre-engine marshal→reth replay can't bridge it.
    let lowest = match recover_reconnect_point(provider, target, MAX_COLD_RECOVER)? {
        ReconnectScan::Reconnect(lowest) => lowest,
        ReconnectScan::TooDeep => {
            return crash_recover_defer_or_fatal(
                provider,
                target,
                upstream.is_some(),
                sync_metrics,
                &format!("reth is > {MAX_COLD_RECOVER} blocks behind its own finalized archive at {target}"),
            );
        }
    };

    // Phase 2: replay [lowest..=target] from the marshal's OWN finalized_blocks
    // archive — BLOCKS-ONLY (Design B′, R3). The seed for height `h` is block
    // `h+1`'s `parent_seed` (the witness), read via a one-element lookahead —
    // the SAME rule the live executor derives with, so a restarted node can
    // never re-execute a height with a different `prev_randao` than the network
    // (F4). Per-height finalization certs are NOT read: a present block with an
    // absent cert is a NORMAL state (an ancestry-finalized height may have no
    // standalone cert anywhere, ever — pre-B′ this was classified as a
    // re-fetchable hole, the upstream could not serve it, and the node could
    // NEVER restart). Certs authenticate a re-fetched MISSING BLOCK only.
    let archive = crate::outer::init_finalized_blocks_archive(ctx, MARSHAL_PARTITION_PREFIX).await;

    let mut parent_hash = provider
        .block_hash(lowest.saturating_sub(1))
        .wrap_err("provider.block_hash at recovery reconnect point")?
        .ok_or_else(|| {
            eyre!(
                "crash-survivor recovery: no reconnect parent below height {lowest}; \
                 re-sync the EL disk from a snapshot"
            )
        })?;
    let mut order = recover_walk_block(
        &archive,
        upstream,
        committees,
        &mut verify_ctx,
        parent_hash,
        l1_checkpoint,
        sync_metrics,
        lowest,
    )
    .await?;
    for h in lowest..=target {
        // The WITNESS for `h` = block `h+1`'s `parent_seed`. For `h < target` the
        // child is the next walk element (fetched once — it becomes the next
        // iteration's `order`). For `h == target` (= the marshal's
        // `last_processed_height`, i.e. a height the executor DERIVED and acked
        // before the crash) the child was necessarily dispatched — and therefore
        // stored — so the archive read succeeds wherever the derive itself once
        // did; a corruption hole falls back to the same #8 re-fetch. The one
        // exception: a PRE-BEACON `target` (its own `parent_seed` is `None`)
        // derives with the agreed digest fallback and needs no child at all.
        let (seed, next_order) = if h == target {
            let child = archive
                .get(Identifier::Index(h + 1))
                .await
                .map_err(|e| eyre!("reading marshal finalized_blocks at height {}: {e}", h + 1))?;
            match child {
                Some(child) => (child.parent_seed, None),
                None if order.parent_seed.is_none() => (None, None),
                None => {
                    let uf = refetch_verified_archive_hole(
                        upstream,
                        committees,
                        &mut verify_ctx,
                        parent_hash,
                        l1_checkpoint,
                        h + 1,
                        "finalized_blocks (witness child)",
                    )
                    .await?;
                    sync_metrics.crash_recover_refetched.inc();
                    (uf.block.parent_seed, None)
                }
            }
        } else {
            let child = recover_walk_block(
                &archive,
                upstream,
                committees,
                &mut verify_ctx,
                parent_hash,
                l1_checkpoint,
                sync_metrics,
                h + 1,
            )
            .await?;
            (child.parent_seed.clone(), Some(child))
        };
        // Witness-downgrade refusal (same agreed-data monotonicity rule as the
        // executor): a block that itself carries a witness sits on a
        // beacon-active link, so its child MUST present one. Deriving with the
        // digest fallback here would re-roll `prev_randao` and FORK the restart.
        ensure!(
            seed.is_some() || order.parent_seed.is_none(),
            "crash-survivor recovery: block {h} is on a beacon-active link but its child \
             presents no parent_seed witness — corrupted archive; refusing a digest-fallback \
             derive (it would fork); re-sync the EL disk from a snapshot"
        );
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
        // Judged by the next derive, not by the response code: VALID and "parent
        // is visible" diverge in BOTH directions — SYNCING during a backfill does
        // nothing, and INVALID can be returned AFTER canonicalization already
        // happened. The typed `ParentHeaderMissing` on the following iteration is
        // the honest signal; `derive_with_visibility_retry` already absorbs the
        // concurrent-devp2p case here.
        if !resp.is_valid() {
            warn!(
                height = h,
                status = ?resp.payload_status,
                "crash-survivor recovery FCU not VALID; the next derive will report \
                 whether the parent became visible"
            );
        }
        // Hand the already-fetched child to the next iteration (each walk
        // element is fetched exactly once). `None` only at `h == target`,
        // where the range is exhausted anyway.
        match next_order {
            Some(next) => order = next,
            None => break,
        }
    }
    drop(archive); // release so MarshalActor::init can re-open the same partition.

    let hash = provider
        .block_hash(target)
        .wrap_err("provider.block_hash after crash-survivor recovery")?
        .ok_or_else(|| {
            eyre!("crash-survivor recovery: block {target} still missing from reth after replay")
        })?;
    Ok(RecoverOutcome::Recovered(hash))
}

/// Operator-supplied per-launch configuration. Keys + JSON-parsed
/// configs arrive pre-loaded (the host crate owns filesystem syscalls
/// and permission checks); the slasher transport arrives pre-built
/// because `PoolTxSink<P, Provider>` carries concrete
/// `reth-transaction-pool` trait bounds that can't compile in this crate.
pub struct DposLayerConfig<D, XC, A, U> {
    pub bls_keypair: ValidatorBlsKeypair,
    pub peer_keypair: commonware_cryptography::ed25519::PrivateKey,
    pub slasher_sink: Arc<dyn SlasherTxSink>,
    /// Evidence-channel bridge to the node's gossip task, which owns both p2p
    /// halves of `EVIDENCE_CHANNEL` ([`crate::slasher::gossip`]).
    pub evidence: crate::slasher::EvidenceBridge,
    pub staking_config: StakingReaderConfig,
    /// Datadir path of the fork-safety halt marker
    /// ([`crate::sync_metrics::SafetyHalt::restoring`]). A marker left by a
    /// previous run brings this node up permanently verify-only — the latch has
    /// no in-process `disengage`, so without it a restart silently cleared a
    /// halt and the node signed again on the same disk. `None` only in tests.
    pub halt_marker: Option<std::path::PathBuf>,
    /// Cert upstream for the single-shot, pre-engine cold-start EL-sync JUMP
    /// ([`crate::cold_start_jump`]). `Some` ⇒ an upstream-configured node
    /// (production-path external joiner / follower): a deep cold-start gap is
    /// fast-forwarded via one FCU + devp2p backfill before the OuterEngine
    /// starts. `None` ⇒ a no-upstream validator: it catches up on the
    /// consensus-plane treadmill instead (no jump). FreshMigration never jumps
    /// (the clean-halt invariant pins its anchor at `dposActivationBlock`).
    pub upstream: Option<U>,
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
    /// Edge-trigger the executor fires on each finalized-advance — the mid-epoch
    /// promotion trigger for the role reconciler (the executor is the sole reth
    /// writer on a validator; it follows the chain by local derivation).
    pub spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// The always-on beacon/DKG plane, built ONCE per process in the node crate
    /// (`build_beacon_plane`) and shared across the follower↔signer phase switch.
    /// The
    /// signer engine is a CONSUMER of its shared `ceremony_store` (the per-epoch
    /// `PK_epoch`/share source) and its artifact ladder rungs, re-uses
    /// its `oracle` (the single network's peer set) + its already-registered
    /// `beacon_metrics`, and CLONES its 5 `MuxHandle`s + `subscribe()`s the vote
    /// backup to wire the OuterEngine's per-promotion sub-channels — it never
    /// re-builds the network, re-spawns the `DkgActor`, re-registers the metrics, or
    /// re-binds `listen`.
    pub beacon_plane: SharedBeaconPlane,
    /// The shared upstream-frontier atomic for the validator-with-upstream re-jump
    /// (Rule Y). Created ONCE in the node crate (`node/dpos.rs::launch_validator_overlay`)
    /// and threaded into BOTH the validator inlet's `LiveFrontierTee.upstream_frontier`
    /// (writer) and this re-jump (reader) — the same inlet⇄executor signal the follower
    /// has. HEIGHT-ONLY, never feeds committee/DKG selection (I6). A no-upstream
    /// validator passes a standalone `0` atomic (nothing writes it; `re_jump` is `None`).
    pub upstream_frontier: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Supervisor handles of the epoch-key agreement instances the beacon plane
    /// starts. Threaded straight through to [`crate::epoch_manager::Actor`], which
    /// owns them and prunes them on the same frontier cutoff as the per-epoch
    /// engines. `None` ⇒ no agreement plane wired.
    pub agreement_intake: Option<mpsc::Receiver<(Epoch, commonware_runtime::Handle<()>)>>,
    /// DEVNET/TEST-ONLY byzantine behaviour (gated behind `dpos-devnet-byzantine`).
    /// Absent — and the field does not exist — in a production build.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
}

/// A plane-owned broker handle for one of the 5 non-beacon channels: the single
/// network's `(Sender, Receiver)` pair are owned by a persistent `Muxer` in the
/// always-on plane (node crate); every promotion CLONES this handle (an `Arc`) and
/// registers fresh sub-channels against the SAME broker. A `SubReceiver`
/// auto-deregisters on drop, so a demoted engine that drops its `SubReceiver`s frees
/// the slots and a re-promoted one re-registers — restart-free re-promotion.
///
/// `MuxHandle::register` takes `&mut self`, so the shared handle is wrapped in
/// `Arc<Mutex<_>>`: each `register` (a boundary-rate control-channel round-trip)
/// locks transiently. The derived `Clone` on `MuxHandle<S, R>` carries a spurious
/// `R: Clone` bound that the move-only `DiscReceiver` does NOT satisfy, so the bare
/// `MuxHandle` is itself un-`Clone`able here — the `Arc` is both the sharing
/// mechanism AND the `Clone` we need for `SharedBeaconPlane`.
pub type PlaneMux = Arc<
    Mutex<
        commonware_p2p::utils::mux::MuxHandle<
            fluentbase_p2p::DiscSender<Context>,
            fluentbase_p2p::DiscReceiver,
        >,
    >,
>;

/// One item the vote Muxer's backup channel surfaces: a vote for an epoch with no
/// registered sub-channel (the network is ahead of us). `(subchannel == epoch, (from,
/// payload))`; the payload is unused by the catch-up hint. Mirrors the mux's
/// `BackupResponse<PublicKey>` so the `EpochManager` backup arm is unchanged.
pub type VoteBackupItem = (u64, (PeerPubkey, commonware_runtime::IoBuf));

/// A re-settable forwarding target: a single mpsc slot the plane re-points to the
/// CURRENTLY-active consumer per promotion. The plane's forwarder drains a move-only
/// source (the vote Muxer's backup receiver) and `try_send`s each item to the parked
/// sender; on demote the receiver drops and the forwarder parks (drops items while no
/// engine is up — a follower needs no catch-up hint). [`subscribe`] hands each
/// promotion a fresh `Receiver`, re-pointing the slot.
#[derive(Clone)]
pub struct ResettableForward<T> {
    slot: Arc<Mutex<Option<mpsc::Sender<T>>>>,
    capacity: usize,
}

impl<T> ResettableForward<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            slot: Arc::new(Mutex::new(None)),
            capacity,
        }
    }

    /// Hand the currently-active consumer a fresh receiver and re-point the
    /// forwarder's sender at it (replacing any prior promotion's). The prior
    /// receiver — already dropped on demote — leaves its sender to fail `try_send`,
    /// so re-pointing is the only state to update.
    pub async fn subscribe(&self) -> mpsc::Receiver<T> {
        let (tx, rx) = mpsc::channel(self.capacity);
        *self.slot.lock().await = Some(tx);
        rx
    }

    /// The shared slot — the plane's forwarder task reads the live target from it.
    pub fn slot(&self) -> Arc<Mutex<Option<mpsc::Sender<T>>>> {
        self.slot.clone()
    }
}

/// The persistent beacon/DKG plane handed DOWN from the node crate's always-on
/// component into each per-promotion signer engine. The node crate owns the single
/// `FluentP2P` (beacon halves + `DkgActor` consume their channel there; the 5
/// non-beacon channels are owned by 5 persistent plane `Muxer`s), the
/// EpochTransition-driven Oracle peer-set, the `dkg_height` clock, and reloads the
/// `ceremony_store` from `<datadir>/beacon/` once at startup; the signer engine
/// reads the SAME shared `Arc`s and CLONES the 5 `MuxHandle`s per promotion. There is
/// exactly ONE network / listen bind / peer set / broker set per process — a
/// demote→re-promote within one process needs no network rebuild (the engine drops
/// its `SubReceiver`s on demote; the next promotion re-registers fresh ones).
#[derive(Clone)]
pub struct SharedBeaconPlane {
    /// The single network's Oracle (the one `Clone` p2p handle), used by the
    /// engine's blocker/provider + its EpochTransition peer-set sink.
    pub oracle: fluentbase_p2p::OracleHandle,
    /// The consensus-facing randomness surface, built by `beacon::build`. The
    /// layer threads it and never opens it.
    pub randomness: Arc<dyn crate::beacon::Randomness>,
    /// The 5 plane-owned non-beacon channel broker handles (vote/cert/resolver are
    /// per-epoch register/deregister; broadcast/marshal register subchannel 0 once
    /// per promotion). Cloned per promotion; the Muxer tasks live in the plane.
    pub vote_mux: PlaneMux,
    pub cert_mux: PlaneMux,
    pub resolver_mux: PlaneMux,
    pub broadcast_mux: PlaneMux,
    pub marshal_mux: PlaneMux,
    /// The vote Muxer's backup re-settable forwarder: the plane owns the move-only
    /// backup receiver and forwards each catch-up item to the currently-active
    /// `EpochManager`; each promotion `subscribe()`s a fresh receiver.
    pub vote_backup: ResettableForward<VoteBackupItem>,
    /// Committee members observed slashed for equivocation. Written by the plane's
    /// tombstone watcher (the sole writer, riding the finalized-height poller);
    /// read by every promoted engine's `FluentApp`. One instance per process, and
    /// the reader must be handed the writer's own handle rather than a second
    /// empty one.
    pub tombstones: crate::slasher::TombstoneSet,
    /// The ordering-vs-DKG clock pair (see [`crate::sync_metrics::PlaneClock`]).
    /// Registered and half-written by the plane's finalized poller; the other
    /// half is written by `FluentApp` off marshal's tip. Same reason as
    /// `tombstones` for arriving from the node crate rather than being defaulted
    /// here: a second instance would be a gauge nothing scrapes.
    pub plane_clock: crate::sync_metrics::PlaneClock,
}

/// Cold-start kind resolved from durable state. Pure function of the inputs
/// so the decision is unit-testable without a node. A deeply-behind node with
/// an upstream re-seeds its `Restart` anchor via the forward [`cold_start_jump`]
/// (no separate kind) rather than anchoring at the EL tip directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColdStartKind {
    /// Empty archive, EL at/inside epoch 0: anchor at the activation block.
    FreshMigration,
    /// Populated archive: resume at its finalized height (real consensus
    /// state always wins).
    Restart,
}

fn resolve_cold_start_kind(
    archive_finalized: u64,
    activation: u64,
    interval: u32,
    cs_finalized: u64,
    has_upstream: bool,
) -> eyre::Result<ColdStartKind> {
    // `0` is the unscheduled sentinel (`setDposActivationBlock` requires a future
    // block, so a live ChainConfig never stores `0` for a genuine migration). At the
    // materialized geometry anchor a real migration always reads nonzero; a `0` here
    // means an unscheduled / mis-configured chain — fail loud rather than anchor DPoS
    // at block 0.
    ensure!(
        activation != 0,
        "dposActivationBlock is 0 (the unscheduled sentinel); DPoS must not launch \
         on an unscheduled ChainConfig"
    );
    if archive_finalized > activation {
        return Ok(ColdStartKind::Restart);
    }
    if cs_finalized >= activation + interval as u64 {
        // EL is past epoch 0 with an empty consensus archive. WITH a sync upstream
        // (plane-tracked or WS) this is a legal deep `Restart`: the pre-engine
        // `cold_start_jump` re-seeds the anchor at the verified frontier (§4.1/§4.3),
        // and the empty-archive caller REQUIRES that jump to LAND (never anchors staking
        // reads on the genesis hash — `dpos.rs::launch`). WITHOUT any upstream it stays
        // FATAL: anchoring DPoS on state of unknown provenance is forbidden.
        ensure!(
            has_upstream,
            "EL is past epoch 0 (finalized {cs_finalized} >= activation {activation} + interval \
             {interval}) with an empty consensus archive and NO sync upstream (not plane-tracked \
             and no --dpos.follower-upstream); refusing to anchor DPoS on a state of unknown \
             provenance. Register+activate the validator (it then joins the plane and \
             cold-start-jumps to the verified frontier) or restore the consensus archive."
        );
        return Ok(ColdStartKind::Restart);
    }
    Ok(ColdStartKind::FreshMigration)
}

/// Whether the single-shot, pre-engine cold-start EL-sync JUMP
/// ([`crate::cold_start_jump`]) is eligible to run. FreshMigration is NEVER
/// eligible: its anchor MUST equal `dposActivationBlock` (the clean-halt
/// invariant) and the pre-DPoS sequencer is production-gated there, so there is
/// no deep gap to jump and a jump would orphan the activation anchor. An
/// upstream is required (a no-upstream validator catches up on the
/// consensus-plane treadmill). The forward-only need-gate (target far enough
/// ahead; landing actually advances) lives INSIDE `cold_start_jump`.
fn cold_start_jump_eligible(kind: ColdStartKind, has_upstream: bool) -> bool {
    kind != ColdStartKind::FreshMigration && has_upstream
}

/// The disposition of a single cold-start [`JumpOutcome`], split so the two
/// single-shot pre-engine call sites can SELF-HEAL a transient stall (#11) rather
/// than re-fuse it to fatal.
enum JumpDisposition {
    /// #11 SELF-HEAL: an EL-sync NET trip (sustained zero devp2p peers / the
    /// backstop ceiling — [`crate::cold_start_jump::JumpOutcome::Stalled`]). The
    /// cold-start path retries FOREVER (a peer may connect) under
    /// `dpos_sync_degraded{reason=no_peers}` — NO backstop exit (Decision A: a
    /// no-peers fatal would restart-storm every honest joiner at once).
    RetryStalled(eyre::Report),
    /// #1 SELF-HEAL: a POST-sync committee-BLS / L1 rejection (`AuthFailed`) — the
    /// CURRENT upstream served a forged/unagreed far-ahead branch. The cold-start
    /// path ROTATES the upstream, backs off, and RE-JUMPS forever under
    /// `dpos_sync_degraded{reason=auth_rotate}` (Decision A — never crash on a
    /// forged UPSTREAM). With ≥2 upstreams `rotate()` routes to the next URL and the
    /// re-jump recovers off the honest source; with a SINGLE upstream `rotate()` is a
    /// no-op so it re-polls the same one degraded (recovery genuinely needs ≥2). The
    /// node never boots on the forged branch (the anchor stays un-advanced).
    RotateAuth(eyre::Report),
    /// A terminal outcome: proceed with `Ok(Some(landing))` / `Ok(None)` (no-op),
    /// or fail closed `Err` (a genuinely-fatal cold-start jump error).
    Done(eyre::Result<Option<(u64, B256, u64)>>),
}

/// Classify [`crate::cold_start_jump::cold_start_jump`]'s typed terminal
/// [`JumpOutcome`] for the two single-shot pre-engine call sites.
///
/// `Landed` ⇒ `Done(Ok(Some(..)))`; `Lagging` ⇒ `Done(Ok(None))`; `BadTarget` /
/// `InvalidTarget` (both a forgeable/reth-rejected structural mismatch — one
/// PRE-sync, one discovered DURING `sync_to`, Rule S) ⇒ `Done(Ok(None))` (boot
/// anyway, never crash on attacker-controlled or upstream-served-bad input);
/// `AuthFailed` ⇒ `RotateAuth` (a POST-sync forged/unagreed branch — rotate +
/// backoff + re-jump forever, was `Done(Err)`);
/// `Stalled` ⇒ `RetryStalled` — the #11 no-peers self-heal (was a fatal re-fuse).
fn classify_jump_outcome(outcome: crate::cold_start_jump::JumpOutcome) -> JumpDisposition {
    use crate::cold_start_jump::JumpOutcome;
    match outcome {
        JumpOutcome::Landed {
            landing,
            hash,
            floor,
        } => JumpDisposition::Done(Ok(Some((landing, hash, floor)))),
        JumpOutcome::Lagging => JumpDisposition::Done(Ok(None)),
        // Rule S: a forgeable PRE-anchor structural mismatch (`payload != digest`)
        // is NON-fatal — boot anyway (the inlet's ordinary pulls cover the residual
        // gap, and a structurally-bad upstream recurs in steady state where Phase 2
        // WARNs + rotates). Distinct from the POST-anchor authenticated rejections.
        JumpOutcome::BadTarget(_e) => JumpDisposition::Done(Ok(None)),
        // Same NON-fatal "boot anyway" treatment as BadTarget: reth itself
        // rejected the served branch as INVALID during the EL-sync attempt
        // (`el_sync_calls >= 1`, unlike BadTarget's PRE-sync `== 0`) — still a
        // bad-upstream signal, not a reason to crash the node.
        JumpOutcome::InvalidTarget(_e) => JumpDisposition::Done(Ok(None)),
        // #11: an EL-sync stall (likely zero devp2p peers) is no longer a
        // cold-start fatal — the call site retries forever + raises the gauge.
        JumpOutcome::Stalled(e) => JumpDisposition::RetryStalled(e),
        // Connected-but-wedged EL pipeline (soak v43): `sync_to` already ERROR-logged
        // the stuck head. At cold-start the node has committed to nothing, so retry
        // forever (same posture as `Stalled`) — a deterministic divergence keeps
        // re-wedging, but the node stays observable + un-advanced rather than crashing.
        JumpOutcome::StalledWithPeers(e) => JumpDisposition::RetryStalled(e),
        // A POST-sync committee-BLS rejection of a
        // canonicalized branch means the CURRENT upstream is forged — rotate + backoff
        // + re-jump forever (was fail-closed `Done(Err)`); never crash on a forged
        // UPSTREAM (Decision A).
        JumpOutcome::AuthFailed(e) => JumpDisposition::RotateAuth(e),
        // #10 L1 fork PRE-ENGINE: the node has not committed to serving anything yet
        // (the anchor stays un-advanced), so rotate + re-jump forever exactly like a
        // forged upstream — a different honest source may serve an L1-consistent
        // branch, and the node never boots on the forked chain. The RUNTIME SafetyHalt
        // (executor `jump_done` arm) is where an L1 fork halts an already-serving node;
        // here there is nothing up to keep alive, so the stay-up posture IS the rotate.
        JumpOutcome::L1Fork(e) => JumpDisposition::RotateAuth(e),
    }
}

/// Run the single-shot cold-start jump with the #11 no-peers + #1 forged-upstream
/// SELF-HEALs: a transient `Stalled` re-attempts FOREVER on the [`EL_SYNC_TICK`]
/// cadence under `dpos_sync_degraded{reason=no_peers}=1` (a peer may connect), and an
/// `AuthFailed` ROTATES the upstream + backs off + re-jumps FOREVER under
/// `{reason=auth_rotate}=1` (a forged UPSTREAM — never crash, Decision A); any
/// terminal outcome resolves via [`classify_jump_outcome`]. Shared by the validator
/// `launch` and `launch_follower` cold-start call sites.
///
/// [`EL_SYNC_TICK`]: crate::cold_start_jump::EL_SYNC_TICK
#[allow(clippy::too_many_arguments)]
async fn cold_start_jump_self_heal<U, C, ES>(
    ctx: &Context,
    sync_metrics: &SyncMetrics,
    anchor: u64,
    upstream: &U,
    committees: &C,
    el: &ES,
    l1_checkpoint: Option<B256>,
    activation: u64,
    jump_ctx: &mut Context,
) -> eyre::Result<Option<(u64, B256, u64)>>
where
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
    ES: crate::cold_start_jump::ElSync,
{
    let mut stalled = false;
    let mut auth_rotated = false;
    loop {
        let outcome = crate::cold_start_jump::cold_start_jump(
            anchor,
            upstream,
            committees,
            el,
            l1_checkpoint,
            activation,
            jump_ctx,
        )
        .await;
        match classify_jump_outcome(outcome) {
            JumpDisposition::RetryStalled(e) => {
                stalled = true;
                warn!(
                    error = ?e,
                    "cold-start EL-sync stalled (likely zero devp2p peers); retrying (no give-up)"
                );
                sync_metrics.degrade(SyncReason::NoPeers);
                ctx.sleep(crate::cold_start_jump::EL_SYNC_TICK).await;
            }
            JumpDisposition::RotateAuth(e) => {
                // #1 SELF-HEAL: the CURRENT upstream served a forged/unagreed
                // far-ahead branch. Rotate to the next upstream (a NO-OP at a single
                // upstream — round-robin len==1), back off, and RE-JUMP forever under
                // `reason=auth_rotate`; never crash on a forged UPSTREAM (Decision A),
                // and never advance the anchor onto the forged branch.
                auth_rotated = true;
                warn!(
                    error = ?e,
                    "cold-start jump target failed committee-BLS / L1 authentication (forged \
                     upstream); rotating upstream + backing off + re-jumping (no give-up)"
                );
                sync_metrics.degrade(SyncReason::AuthRotate);
                upstream.rotate().await;
                ctx.sleep(crate::cold_start_jump::EL_SYNC_TICK).await;
            }
            JumpDisposition::Done(res) => {
                if stalled {
                    sync_metrics.recover(SyncReason::NoPeers);
                }
                if auth_rotated {
                    sync_metrics.recover(SyncReason::AuthRotate);
                }
                return res;
            }
        }
    }
}

/// #17 SELF-HEAL visibility belt: a cold-start / follower landing read of a block
/// reth JUST materialized (via an EL-sync jump / devp2p canonicalization) can
/// transiently return `None` — reth-2.2 canonicalizes on the engine-tree thread a
/// few ms before provider reads see the block (the same race
/// `derive_with_visibility_retry` absorbs on the derive side). Retry `read` on the
/// same 100 ms/10 s cadence under `dpos_sync_degraded{reason=landing_wait}=1`;
/// fatal ONLY after the belt expires (a materialized-but-missing read is then the
/// genuine data-loss fault). Bounded (not retry-forever): unlike an external
/// peer/anchor wait, a landing that never materializes past its own EL-sync is a
/// local fault, not a correlated one.
async fn read_with_visibility_belt<T, F, C>(
    ctx: &C,
    sync_metrics: &SyncMetrics,
    what: &str,
    mut read: F,
) -> eyre::Result<T>
where
    C: commonware_runtime::Clock,
    F: FnMut() -> eyre::Result<Option<T>>,
{
    // Same constants as `derive_with_visibility_retry` (application.rs).
    const RETRY: Duration = Duration::from_millis(100);
    const DEADLINE: Duration = Duration::from_secs(10);
    let deadline = ctx.current() + DEADLINE;
    let mut waited = false;
    loop {
        if let Some(v) = read()? {
            if waited {
                sync_metrics.recover(SyncReason::LandingWait);
            }
            return Ok(v);
        }
        if ctx.current() >= deadline {
            if waited {
                sync_metrics.recover(SyncReason::LandingWait);
            }
            return Err(eyre!(
                "reth does not hold {what} after the visibility belt ({DEADLINE:?}) expired"
            ));
        }
        waited = true;
        sync_metrics.degrade(SyncReason::LandingWait);
        ctx.sleep(RETRY).await;
    }
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

pub struct DposLayerHandle {
    pub consensus_handle: Handle<()>,
    /// Marshal mailbox clone for the node-side cert feed/RPC (by-height
    /// `get_finalization`+`get_block`). The node calls `feed_handle.set_marshal`
    /// with this once `launch` returns — keeping node types out of consensus.
    pub cert_mailbox: crate::outer::MarshalMailbox,
    /// Layer-internal tasks the HOST must supervise alongside
    /// `consensus_handle`, each with the label the host's exit log prints.
    ///
    /// The runtime runs with `with_catch_panics(true)` and a commonware
    /// `Handle` has NO `Drop` impl, so a DROPPED handle detaches its task: a
    /// panic in it logs one line, resolves nothing, and the node keeps passing
    /// liveness checks with the subsystem dead. Handing the handle up instead
    /// lands it in `dpos.rs::supervise`, where a panic surfaces as
    /// `Err(Error::Exited)` → shared-token cancel → node down (the SAME fatal
    /// semantics `("inlet", h)` already gets on the validator path).
    ///
    /// Appended into the host's `Vec<SupervisedHandle>` — same tuple shape.
    pub supervised: Vec<(&'static str, Handle<()>)>,
    /// Layer-internal tasks the host must LET FINISH on a graceful stop, each
    /// with the label the host's drain log prints. Today: the durable
    /// seed-journal writer, whose last act is to append and fsync whatever the
    /// store queued but had not written yet.
    ///
    /// Same tuple shape as `supervised`, OPPOSITE semantics, and the two must
    /// never be merged: a `supervised` handle resolving means "a subsystem
    /// died, cancel the node", while one of these resolving means "the task
    /// finished the work it owed, shutdown may proceed". Putting a drain task
    /// in `supervised` would make its normal completion look like a crash;
    /// putting a supervised task here would make its crash look like success.
    ///
    /// Every one of these tasks is spawned OUTSIDE the consensus engine's
    /// supervision subtree, because the host awaits them only AFTER aborting
    /// that engine — see `crate::outer::OuterBuilder::build`.
    pub drain_on_shutdown: Vec<(&'static str, Handle<()>)>,
    /// Serve one held epoch-key artifact over `consensus_getEpochArtifact`, so a
    /// TIER-2 follower obtains `PK_epoch` from this node exactly as it obtains
    /// certificates from it.
    ///
    /// `Some` on the FOLLOWER path only, and the asymmetry is not an oversight: a
    /// follower's artifact store is created inside `beacon::for_follower`, below
    /// this crate boundary, so this handle is the only way out. A validator's
    /// store belongs to its always-on beacon plane, which the node builds itself
    /// — it reads `Beacon::artifact_bytes` directly and never needs this field.
    pub artifact_bytes: Option<crate::beacon::ArtifactSource>,
}

/// Read `committee[epoch]` for the follower's boundary trigger. `None` ⇒ not
/// readable yet — no executed anchor, or the epoch's committee not committed at
/// it — which the trigger treats as "retry on the next finalized block", never as
/// an empty committee.
type FollowerCommitteeAt = Arc<dyn Fn(u64) -> Option<ValidatorSetSnapshot> + Send + Sync>;

/// Hand one `(epoch, snapshot)` to the epoch manager's boundary receiver.
/// `false` ⇒ the receiver is gone (the manager exited); the trigger stops.
type FollowerBoundaryDeliver = Arc<
    dyn Fn(Epoch, ValidatorSetSnapshot) -> futures::future::BoxFuture<'static, bool> + Send + Sync,
>;

/// One step of the follower's epoch-boundary trigger: deliver `(epoch, snapshot)`
/// for the epoch the finalized stream has entered, at most once per epoch.
/// Returns whether the trigger should keep running.
///
/// A validator gets its boundary deliveries from `EpochTransition`, which rides
/// the beacon plane. A follower has no plane, so it derives the same delivery
/// from the finalized `OrderBlock`s its own marshal reports — the seam
/// `boundary_hook` already exists for. Three things downstream need it and all
/// three were dead on a follower without it: `soft_enter` registering the current
/// epoch's verify-only scheme (without which a resolver-delivered cert for any
/// epoch above the cold-start one finds NO scheme and the marshal answers the
/// fetch `true` without storing — a re-request loop that never closes),
/// `highest_entered_epoch` (the repair sweep's only frontier evidence here, since
/// the vote-backup arm that feeds the corroborated one is parked), and
/// `latest_live` (the snapshot hash the sweep's boundary FETCH authenticates its
/// committee read at).
///
/// **`last_delivered` advances only after a delivery.** An unreadable committee
/// must leave the epoch unconsumed: `committee[E]` is committed during `E-1` but
/// the read runs at the EL-finalized hash, which trails the ordering-finalized
/// height by `K`, so the first blocks of `E` can legitimately read back nothing.
/// Consuming the epoch there would skip its registration until the NEXT boundary.
///
/// **Epochs this step skipped entirely are deliberately not back-filled**, and
/// they are skipped two ways. A FLOOR MOVE (a cold start or a re-jump walks the
/// marshal floor forward) dispatches no block of the skipped epoch, so no
/// certificate of it is ever fetched and there is nothing to register a scheme
/// for. DRIVER LAG is the case where blocks WERE dispatched: this step reads one
/// latest height and derives one epoch from it, so an epoch that both begins and
/// ends while the step sits inside `committee_at`'s EVM snapshot read is passed
/// over with its blocks already reported — and the repair sweep cannot cover for
/// it either, because its work list is the REGISTERED schemes and a skipped epoch
/// registers none. The cost is the one named above, scoped to that epoch: a
/// resolver-delivered certificate for it finds no scheme and the marshal answers
/// the fetch `true` without storing.
///
/// Accepted rather than back-filled because reaching it means the finalized
/// height advancing a whole `interval` of blocks inside one state read — at the
/// production `epochBlockInterval` of 86 400 that is a day of chain against a
/// single snapshot read — and a back-fill would pay for that reachability with a
/// committee read per skipped epoch on exactly the read that is, by hypothesis,
/// the slow one.
async fn enter_finalized_epoch(
    last_delivered: &mut Option<u64>,
    finalized_height: u64,
    activation: u64,
    interval: u32,
    committee_at: &FollowerCommitteeAt,
    deliver: &FollowerBoundaryDeliver,
) -> bool {
    let epoch =
        fluentbase_staking_reader::reader::epoch_of_block(finalized_height, interval, activation);
    if *last_delivered >= Some(epoch) {
        return true;
    }
    let Some(snap) = committee_at(epoch) else {
        return true;
    };
    if !deliver(Epoch::new(epoch), snap).await {
        return false;
    }
    *last_delivered = Some(epoch);
    true
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
    pub async fn launch<Provider, EvmConfig, BeaconEngine, D, XC, A, U>(
        ctx: Context,
        reth: RethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: DposLayerConfig<D, XC, A, U>,
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
        U: crate::cert_follow::CertUpstream,
    {
        let DposLayerConfig {
            bls_keypair,
            peer_keypair,
            slasher_sink,
            evidence,
            staking_config,
            halt_marker,
            upstream,
            deriver,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            feed,
            spawn_unblocked,
            beacon_plane,
            upstream_frontier,
            agreement_intake,
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
        } = cfg;

        // The plane owns the 5 persistent Muxers; this promotion CLONES their handles
        // (to register fresh per-epoch / subchannel-0 routes against the SAME broker
        // tasks) and `subscribe()`s a fresh vote-backup receiver. No raw move-only
        // halves are consumed here, so a later demote→re-promote re-clones cleanly.
        let SharedBeaconPlane {
            oracle,
            randomness,
            vote_mux,
            cert_mux,
            resolver_mux,
            broadcast_mux,
            marshal_mux,
            vote_backup,
            tombstones,
            plane_clock,
        } = beacon_plane;
        let vote_backup_rx = vote_backup.subscribe().await;

        let RethHandle {
            provider,
            evm_config,
            beacon_engine_handle,
            chain_id,
            canonical_state,
            genesis_hash,
            peer_count,
        } = reth;

        // Self-heal stuck-detector: registered ONCE per launch (mirrors
        // `BeaconMetrics`) and cloned into the cold-start + boundary-hook self-heal
        // loops below. `dpos_sync_degraded{reason}` is the observable that replaces
        // the removed cold-start/boundary `process::exit`s (Decision A).
        let sync_metrics = SyncMetrics::default();
        sync_metrics.register(&ctx);
        // Fork-safety latch (Phase 3): shared across the executor, epoch_manager, and
        // the OuterEngine supervisor. Engaging it (result divergence / EL Invalid /
        // L1 fork) halts participation while the node stays up + observable.
        // A marker left by a previous run re-engages it HERE, before the first
        // consensus event, so a halted node never comes back as a signer.
        let safety_halt = match halt_marker {
            Some(path) => crate::sync_metrics::SafetyHalt::restoring(sync_metrics.clone(), path),
            None => crate::sync_metrics::SafetyHalt::new(sync_metrics.clone()),
        };

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

        // Reth's `BlockchainProvider::with_latest` populates
        // `canonical_state.finalized_block` from
        // `ChainState::LastFinalizedBlock` during node init, so on a
        // graceful-shutdown restart `get_finalized_num_hash()` returns
        // `Some(disk_finalized.num_hash())`. The genesis fallback
        // handles the pristine-network case (no FCU yet).
        // Head (`head_num`/`head_hash`) is re-read AFTER the cold-start JUMP
        // below (the jump drives reth's canonical head forward), so only the
        // finalized pair is needed here for the discriminator.
        let (cs_finalized, cs_finalized_hash, _head_num, _head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

        // Activation origin + epoch length, read EARLY (at the reth-restored
        // finalized hash) — the cold-start discriminator below needs them before
        // `initial_epoch`. `dposActivationBlock` is immutable and
        // `epochBlockInterval` is governance-stable across the short
        // migration/restart window, so reading at `cs_finalized_hash` matches
        // reading at the resumed height. The finalized hash is the one reth
        // pre-populates into `canonical_in_memory_state` during init (via
        // `with_latest`), so the read hits the ready in-memory state arm — a
        // by-NUMBER hash would go to the DB historical arm and can revert before
        // it materializes.
        let dpos_activation_block = reader.dpos_activation_block(cs_finalized_hash)?;
        let interval = reader.epoch_block_interval(cs_finalized_hash)?;
        let epoch_length_blocks =
            NonZeroU64::new(interval as u64).ok_or_eyre("epoch_block_interval must be > 0")?;

        // Cold-start discriminator (restart vs fresh migration). The marshal's
        // durable application-metadata is the signal: an empty store (height <=
        // activation) is a fresh sequencer→DPoS migration — unless the EL overshot
        // epoch 0, which is fatal (state of unknown provenance). A populated
        // store is a restart of an already-migrated node, which MUST resume at
        // its real finalized height so the scheme cascade starts at the correct
        // epoch. A deeply-behind restart with an upstream then fast-forwards via
        // the forward `cold_start_jump` below.
        let archive_finalized =
            read_consensus_archive_last_finalized(&ctx, MARSHAL_PARTITION_PREFIX).await?;
        let kind = resolve_cold_start_kind(
            archive_finalized,
            dpos_activation_block,
            interval,
            cs_finalized,
            upstream.is_some(),
        )?;
        // The legal deep-`Restart` arm reached with an EMPTY archive (archive at/below
        // activation) anchors at `archive_finalized` = genesis until the jump lands, so
        // it MUST land: an un-landed jump would leave the staking reads below
        // pointed at the genesis hash, where a runtime-deployed ChainConfig is
        // codeless → an opaque "evm read call reverted" crash. A populated Restart (real
        // archived finalized block) has no such hazard — the anchor is already readable.
        let empty_archive_requires_landed_jump =
            kind == ColdStartKind::Restart && archive_finalized <= dpos_activation_block;
        // #8/#12 self-heal: set when the pre-engine crash-survivor replay DEFERRED
        // to the post-engine devp2p EL-sync (anchoring at reth's tip). Cleared once
        // the eligible jump below fast-forwards reth (the `crash_recover` gauge).
        let mut crash_recover_deferred = false;
        let (mut latest_finalized, mut latest_finalized_hash) = if kind
            == ColdStartKind::FreshMigration
        {
            // FRESH MIGRATION: anchor ≡ block@dposActivationBlock; wait for reth
            // to hold it, hash derived locally (canonical at a finalized height).
            // Checkpoint-provisioned EL-ahead start is deferred to Phase 2/β — a
            // node-local EL head as genesis would diverge cross-node.
            let hash =
                wait_for_activation_block(&ctx, &provider, dpos_activation_block, &sync_metrics)
                    .await?;
            (dpos_activation_block, hash)
        } else {
            // RESTART (already migrated): resume at the consensus archive's
            // finalized height.
            match provider.block_hash(archive_finalized)? {
                Some(hash) => (archive_finalized, hash),
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
                    //
                    // The committee source authenticates a #8 below-floor re-fetch
                    // (`committee[E]` read at the already-recovered parent state); the
                    // validator path carries no L1 checkpoint (the on-chain committee
                    // at the local finalized tip is the trust anchor — same as the jump).
                    let recover_committees = crate::cert_inlet::RethCommitteeSource::new(
                        RethStakingStateReader::new(
                            provider.clone(),
                            evm_config.clone(),
                            staking_config.clone(),
                        ),
                        chain_id,
                        {
                            let p = provider.clone();
                            Arc::new(move || {
                                let n = p.finalized_block_number().ok()??;
                                p.block_hash(n).ok().flatten()
                            })
                        },
                    );
                    match recover_finalized_tail_into_reth(
                        &ctx,
                        &beacon_engine_handle,
                        &provider,
                        &deriver,
                        archive_finalized,
                        upstream.as_ref(),
                        &recover_committees,
                        None,
                        &sync_metrics,
                    )
                    .await?
                    {
                        RecoverOutcome::Recovered(hash) => (archive_finalized, hash),
                        RecoverOutcome::DeferToElSync { gap } => {
                            // #12 (reth deeply behind an INTACT archive): the pre-engine
                            // replay is capped, so anchor at reth's ACTUAL tip and let
                            // the eligible `cold_start_jump` below devp2p-backfill the EL
                            // (`has_upstream` was asserted inside recover). #8 holes never
                            // reach here — they heal inline via the by-height re-fetch.
                            crash_recover_deferred = true;
                            let best = provider.best_block_number()?;
                            info!(
                                gap,
                                reth_best = best,
                                finalized_target = archive_finalized,
                                "crash-survivor recovery deferred to devp2p EL-sync; anchoring at \
                                 reth's tip for the pre-engine jump"
                            );
                            let best_hash = read_with_visibility_belt(
                                &ctx,
                                &sync_metrics,
                                &format!("the crash-recover reth tip {best}"),
                                || provider.block_hash(best).wrap_err("reading reth tip hash"),
                            )
                            .await?;
                            (best, best_hash)
                        }
                    }
                }
            }
        };

        // Single-shot, forward-only, pre-engine EL-sync JUMP. For an
        // upstream-configured, deeply-behind node (production-path external
        // joiner / future follower) the resolved anchor can be millions of
        // blocks below the live frontier; the JUMP fast-forwards reth via one
        // FCU + devp2p backfill so the inlet+marshal then close the residual gap
        // by ordinary pulls. Gated:
        //   - `kind != FreshMigration` — a fresh migration MUST anchor at
        //     `dposActivationBlock` (the clean-halt invariant below); it never
        //     jumps.
        //   - `upstream.is_some()` — a no-upstream validator catches up on the
        //     consensus-plane treadmill (epoch_manager soft-enter), NOT here.
        // SAFETY (single writer): this runs BEFORE `OuterBuilder::build` (and
        // thus before the executor task starts) — mutually exclusive with the
        // executor, the SAME property `recover_finalized_tail_into_reth` relies
        // on. `sync_to` issues exactly one read-side fast-forward FCU; it is a
        // cold-start prep path, never a concurrent second reth writer.
        let mut jumped_marshal_floor: Option<Height> = None;
        if cold_start_jump_eligible(kind, upstream.is_some()) {
            // `upstream.is_some()` is the eligibility gate above, so this unwrap
            // is total — destructure via `if let` to keep `up` borrowed.
            if let Some(up) = &upstream {
                let committees = crate::cert_inlet::RethCommitteeSource::new(
                    RethStakingStateReader::new(
                        provider.clone(),
                        evm_config.clone(),
                        staking_config.clone(),
                    ),
                    chain_id,
                    {
                        let p = provider.clone();
                        Arc::new(move || {
                            let n = p.finalized_block_number().ok()??;
                            p.block_hash(n).ok().flatten()
                        })
                    },
                );
                let el = crate::cold_start_jump::RethElSync::new(
                    ctx.clone(),
                    provider.clone(),
                    beacon_engine_handle.clone(),
                    dpos_activation_block,
                    peer_count.clone(),
                );
                // The post-sync cert `verify()` inside `verify_jump_authenticated`
                // needs a `&mut CryptoRngCore`; clone `ctx` (a cheap handle) so the
                // move does not consume the launcher's own `ctx`.
                let mut jump_ctx = ctx.clone();
                loop {
                    if let Some((h, hash, floor)) = cold_start_jump_self_heal(
                        &ctx,
                        &sync_metrics,
                        latest_finalized,
                        up,
                        &committees,
                        &el,
                        // No L1 checkpoint on the validator path: the deep jump is
                        // authenticated trustlessly by the POST-sync committee read at
                        // the landing (`verify_jump_authenticated`), which fails closed
                        // if the upstream serves a forged/unagreed branch. The L1
                        // arg is only the operator-gated fallback for the degenerate
                        // "committee unreadable even at the landing" case.
                        None,
                        dpos_activation_block,
                        &mut jump_ctx,
                    )
                    .await?
                    {
                        latest_finalized = h;
                        latest_finalized_hash = hash;
                        jumped_marshal_floor = Some(Height::new(floor));
                    }
                    // #4 retry-for-upstream: an empty-archive legal-Restart MUST land
                    // the jump before it can read staking state (the genesis ChainConfig
                    // is codeless), but a NON-landed jump (Lagging — frontier within
                    // JUMP_THRESHOLD, or no plane peer has served a frontier yet, e.g.
                    // the node booted before peers tracked it at their next epoch
                    // boundary) is NOT fatal (Decision A: a no-upstream/no-frontier exit
                    // would restart-storm every honest joiner). Re-attempt FOREVER under
                    // dpos_sync_degraded{reason=awaiting_upstream}=1 until a committee
                    // peer serves a frontier above the threshold — the plane IS the path
                    // to BFT (the removed guard already said "retry once plane-tracked").
                    // A POPULATED Restart / fresh migration needs no landing → breaks on
                    // the first pass.
                    if empty_archive_requires_landed_jump && jumped_marshal_floor.is_none() {
                        sync_metrics.degrade(SyncReason::AwaitingUpstream);
                        warn!(
                            "empty consensus archive + EL past epoch 0: cold-start jump has not \
                             landed yet (no plane peer served a frontier above the jump \
                             threshold); parked-degraded, retrying (no give-up — retry once \
                             plane-tracked)"
                        );
                        ctx.sleep(crate::cold_start_jump::EL_SYNC_TICK).await;
                        continue;
                    }
                    break;
                }
                if jumped_marshal_floor.is_some() {
                    sync_metrics.recover(SyncReason::AwaitingUpstream);
                }
            }
        }

        // #12: the eligible jump above devp2p-fast-forwarded reth from its tip, so
        // the pre-engine crash-recover EL gap is closed (the archive was INTACT — no
        // consensus-store repair needed, unlike a #8 hole, which already healed
        // inline during the replay). Clear the gauge.
        if crash_recover_deferred {
            sync_metrics.recover(SyncReason::CrashRecover);
            sync_metrics.crash_recover_gap_blocks.set(0);
        }

        // Empty-archive legal-Restart guard (§4.1): the #4 retry loop above re-attempts
        // the jump FOREVER until it lands whenever `empty_archive_requires_landed_jump`,
        // so by construction `jumped_marshal_floor` is `Some` here for that case — the
        // node NEVER reaches the staking reads below (which would hit the codeless
        // genesis ChainConfig) on an un-landed anchor. This residual fatal is therefore
        // PATH-LESS by construction: it can only fire if the empty-archive case were
        // reached without an upstream (so the retry loop never ran), which
        // `resolve_cold_start_kind` forbids (empty-archive Restart REQUIRES an upstream,
        // the `has_upstream` ensure). Kept as a fail-closed guard against a future
        // refactor breaking that invariant.
        if empty_archive_requires_landed_jump && jumped_marshal_floor.is_none() {
            return Err(eyre!(
                "empty consensus archive with an EL past epoch 0 and NO upstream to jump from — \
                 path-less by construction (resolve_cold_start_kind requires an upstream for this \
                 case). Refusing to run staking reads against the genesis hash (a runtime-deployed \
                 ChainConfig is codeless there)."
            ));
        }

        // Read the EL head AFTER a possible jump: `sync_to` drives reth's
        // canonical head forward via devp2p backfill, so a pre-jump snapshot
        // would be stale. The clean-halt invariant below is FreshMigration-only
        // (which never jumps), so it still sees the un-jumped head.
        let (_cs_fin, _cs_fin_hash, head_num, head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

        // Read AFTER the crash-survivor recovery + jump above: both import the
        // missing reth tail, and a pre-recovery snapshot would make the executor
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
        // no orphaned sequencer-era tail to reconcile. A mismatch means the gate did not run
        // (mis-set chain-config, an ungated node, or a hand-rolled migration): fail
        // loud at cold-start rather than wedge silently in the executor ancestor-FCU
        // guard.
        if kind == ColdStartKind::FreshMigration {
            ensure!(
                head_hash == latest_finalized_hash,
                "fresh migration but reth head {head_num} ({head_hash:?}) != activation \
                 anchor {latest_finalized} ({latest_finalized_hash:?}); the sequencer \
                 was not production-gated at dposActivationBlock — refusing to anchor DPoS \
                 on an orphaned tail"
            );
        }
        let (initial_head_num, initial_head_hash) = (head_num, head_hash);

        let initial_epoch_u64 = fluentbase_staking_reader::reader::epoch_of_block(
            latest_finalized,
            interval,
            dpos_activation_block,
        );

        // Enforce the node ↔ contract invariant
        //   `activeValidatorsLength <= fluentbase_p2p::MAX_COMMITTEE_SIZE`.
        let active_validators_length = reader
            .active_validators_length(latest_finalized_hash)
            .wrap_err("failed reading Staking.activeValidatorsLength")?;
        if (active_validators_length as u64) > fluentbase_p2p::constants::MAX_COMMITTEE_SIZE {
            return Err(eyre!(
                "Staking.activeValidatorsLength ({}) exceeds \
                 fluentbase_p2p::constants::MAX_COMMITTEE_SIZE ({}). Node ↔ contract \
                 cap drift detected — bump MAX_COMMITTEE_SIZE in \
                 crates/dpos/p2p/src/constants.rs AND MAX_ACTIVE_VALIDATORS_LENGTH in \
                 the staking module's contracts/staking/src/consts.rs in the SAME PR, \
                 then redeploy/upgrade.",
                active_validators_length,
                fluentbase_p2p::constants::MAX_COMMITTEE_SIZE,
            ));
        }

        info!(
            chain_id,
            interval,
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
                 `commitEpochCommittee` is a system call the block producer issues \
                 from pre-execution and is SYSTEM_CALLER-only, so there is no \
                 operator command that fixes this. What resolves it is on-chain \
                 state plus block production: `getDposActivationBlock()` must be \
                 scheduled, the registry must hold at least {min} activated \
                 validators with consensus keys (the commit reverts \
                 ERR_COMMITTEE_TOO_SMALL below that), and the producer must then \
                 have advanced far enough for the commit to land and finalize. \
                 Relaunch once the committee is readable at the finalized block.",
                min = fluentbase_staking_reader::reader::MIN_COMMITTEE_LENGTH,
            );
        }

        // The single `FluentP2P` is built ONCE per process by the node crate's
        // always-on beacon plane (and stays up across the follower↔signer switch);
        // this signer engine consumes a CLONE of that one network's `oracle` plus
        // CLONES of the 5 plane-owned non-beacon `MuxHandle`s. It never re-binds
        // `listen`, rebuilds a `Muxer`, or consumes a raw channel half — so a
        // demote→re-promote within one process re-clones cleanly (no network rebuild).

        // Bridge channel: boundary triggers from EpochTransition queue here;
        // a forwarder task (spawned after build) drains bridge_rx →
        // outer_boundary_tx. Built BEFORE EpochTransition so boundary_tx is
        // wired at construction — eliminates the post-build
        // set_boundary_sender race window.
        let (bridge_tx, mut bridge_rx) =
            mpsc::channel::<(u64, fluentbase_staking_reader::reader::ValidatorSetSnapshot)>(64);

        // Wire staking-reader ↔ p2p: EpochTransition consumes the (shared) Oracle as
        // PeerSetSink. The persistent plane's own EpochTransition also tracks this
        // Oracle's peer set so connectivity persists across the follower phase; both
        // compute the identical `active_registry_peers ∪ committee[E+1]` union, so a
        // double `track` of the same (epoch, set) is idempotent.
        let provider_for_et = provider.clone();
        let mut epoch_transition = EpochTransition::new(
            reader,
            oracle.clone(),
            fluentbase_p2p::constants::MAX_REGISTRY_PEER_SET as usize,
            Some(bridge_tx.clone()),
            Arc::new(move |n| executed_state_hash(&provider_for_et, n)),
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
        // and N for sequencer→DPoS migration (the sequencer's last finalised height read from
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
        let genesis_block = anchor_order_block(&genesis_sealed)?;

        // Move EpochTransition into Arc<Mutex<_>> so the boundary_hook
        // closure can call back into it from any thread.
        let et_arc = Arc::new(Mutex::new(epoch_transition));

        // Boundary hook: fires for every `Update::Block`. Spawns
        // fire-and-forget via `ctx.spawn` (NOT `tokio::spawn`, which would
        // depend on the implicit `tokio::Handle::current()` contract under
        // commonware-tokio). The live-DKG epoch clock no longer rides this
        // hook — the always-on plane (node crate) owns the `dkg_height` stream
        // off a persistent finalized-height source, so the `DkgActor` keeps
        // ticking across the follower phase where this signer hook does not run.
        let consecutive_errors = Arc::new(AtomicU32::new(0));
        let et_for_hook = et_arc.clone();
        let ctx_for_hook = ctx.with_label("boundary_hook");
        let errors_for_hook = consecutive_errors.clone();
        let sync_metrics_for_hook = sync_metrics.clone();
        // External wedge detection replacing the deleted give-up: the height of a
        // parked boundary (0 = none). The park has NO internal give-up — a
        // durably-frozen node is caught by a Prometheus `!= 0 for > Xm` alert +
        // the periodic warn! below + the harness recover-stall, never a local
        // counter/clock. Signer-plane twin of the executor cert-budget park's
        // `deferred_height`.
        let parked_boundary_height = Gauge::<i64>::default();
        ctx_for_hook.register(
            "parked_boundary_height",
            "Height of the epoch boundary PARKED awaiting EL state materialization (0 = none). \
             A sustained non-zero value flags a durably-wedged node.",
            parked_boundary_height.clone(),
        );
        // #16: consecutive on_finalized errors, exposed as a gauge instead of a
        // 3-error `shutdown.cancel()`. A boundary-read failure is typically
        // CORRELATED (bad staking state every validator reads) — a fatal here would
        // restart-storm the whole set at once (Decision A). Retry-forever-degraded;
        // a sustained non-zero value (paired with `dpos_sync_degraded{reason=
        // boundary_hook}`) is the external wedge signal.
        let on_finalized_consecutive_errors = Gauge::<i64>::default();
        ctx_for_hook.register(
            "on_finalized_consecutive_errors",
            "Consecutive epoch-boundary on_finalized errors (0 = healthy). Retried-degraded \
             forever (no shutdown); a sustained non-zero value flags a wedged boundary read.",
            on_finalized_consecutive_errors.clone(),
        );
        // The re-poke loop below is the ONE task on this path that genuinely
        // cannot hand a handle to the node supervisor: `enter_boundary` is an
        // `Arc<dyn Fn(u64)>` called from the delivery adapter and from the
        // executor's re-jump landing, spawning one loop PER boundary entry —
        // there is no return path and no fixed set of handles to register. So
        // its death is made LOUD instead: the panic is caught right at the loop
        // (`with_catch_panics(true)` otherwise reduces it to one log line, and a
        // dropped handle detaches the task) and ticked here. Any non-zero value
        // means an epoch boundary lost its re-poke driver — pair it with
        // `parked_boundary_height != 0`, which stays stuck at the height the
        // dead loop was driving.
        let boundary_repoke_panics: Counter = Counter::default();
        ctx_for_hook.register(
            "boundary_repoke_task_panics",
            "Epoch-boundary re-poke tasks killed by a panic (caught by the runtime's \
             catch_panics, so NOT fatal on its own). Non-zero ⇒ a parked boundary has no \
             driver left; expect parked_boundary_height to stay pinned.",
            boundary_repoke_panics.clone(),
        );
        let parked_gauge_for_hook = parked_boundary_height.clone();
        let errors_gauge_for_hook = on_finalized_consecutive_errors.clone();
        // The epoch-entry seam, keyed on a HEIGHT and not on a delivered block, so the
        // steady-state re-jump can drive the SAME entry the delivery path drives without
        // synthesising an `Update::Block`: that update carries an `Exact` ack the executor
        // must fire, and a dropped ack trips marshal's supervisor cascade. The hook only ever
        // read `block.height` anyway.
        let enter_boundary: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |number: u64| {
            let et = et_for_hook.clone();
            let ctx_task = ctx_for_hook.clone();
            let errors = errors_for_hook.clone();
            let parked_gauge = parked_gauge_for_hook.clone();
            let errors_gauge = errors_gauge_for_hook.clone();
            let sync_metrics = sync_metrics_for_hook.clone();
            let repoke_panics = boundary_repoke_panics.clone();
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
                // (catch-up deadlock).
                //
                // NO internal give-up (mirrors the cert-budget executor
                // park): re-poke at PENDING_RETRY_BACKOFF until the park
                // CLEARS (state materialized) or a real error surfaces. A
                // deep re-jump PIPELINE backfill runs up to
                // EL_SYNC_BACKSTOP_CEILING (6 h), which any fixed limit would
                // exhaust — abandoning the last deliverable boundary and
                // wedging epoch E+1 (a recover-stall by a second route). A
                // best_block_number()-progress gate is deliberately NOT used:
                // it is FROZEN for the whole backfill (the exact bug
                // scenario). Liveness against a genuinely-wedged node is
                // EXTERNAL — the parked_boundary_height gauge (+ periodic
                // warn) and the harness recover-stall deadline, never a local
                // counter/clock.
                // LOUD-EXIT rather than supervised (see the
                // `boundary_repoke_task_panics` counter above for why no handle
                // can reach the node supervisor from here): catch the unwind at
                // the loop so a panic in the only driver of THIS boundary becomes
                // a counter tick + an error!, instead of the single anonymous
                // "task panicked" line `with_catch_panics(true)` reduces it to.
                // Only a panic takes the arm below — a clean `break` returns
                // `Ok` and an abort never resolves at all.
                use futures::FutureExt as _;
                let repoke = std::panic::AssertUnwindSafe(async move {
                    let mut pokes = 0u64;
                    loop {
                        let outcome = {
                            let mut et_guard = et.lock().await;
                            et_guard.on_finalized(number).await
                        };
                        match outcome {
                            // Any Ok is a successful boundary read — clear the
                            // consecutive-error streak + the degraded gauge.
                            Ok(TransitionOutcome::EpochAdvanced(_) | TransitionOutcome::Intra) => {
                                if errors.swap(0, Ordering::Relaxed) != 0 {
                                    errors_gauge.set(0);
                                    sync_metrics.recover(SyncReason::BoundaryHook);
                                }
                            }
                            // #16 SELF-HEAL (retry-forever-degraded, Decision A): a
                            // boundary-read failure is typically correlated (bad
                            // staking state every validator reads), so a fatal here
                            // would crash all validators at once. Raise the gauges,
                            // back off, and re-attempt on_finalized — NEVER break/
                            // shutdown (a break would wedge the last-deliverable-
                            // boundary catch-up with no further delivery to re-fire).
                            Err(e) => {
                                let count = errors.fetch_add(1, Ordering::Relaxed) + 1;
                                errors_gauge.set(count as i64);
                                sync_metrics.degrade(SyncReason::BoundaryHook);
                                error!(
                                    block_number = number,
                                    consecutive_errors = count,
                                    error = ?e,
                                    "epoch_transition.on_finalized failed; retrying-degraded \
                                     (no shutdown — a correlated boundary-read failure must not \
                                     crash all validators)"
                                );
                                ctx_inner
                                    .sleep(
                                        fluentbase_staking_reader::epoch_transition::PENDING_RETRY_BACKOFF,
                                    )
                                    .await;
                                continue;
                            }
                        }
                        // BENIGN MISATTRIBUTION: `enter_boundary` spawns one of these
                        // loops per call and both the delivery adapter and the
                        // executor's re-jump landing can call it, so the value read
                        // here may have been parked by the OTHER task. The gauge and
                        // the every-N-pokes warn can therefore credit a park to the
                        // wrong spawn. Nothing is lost by it: the slot is global, both
                        // loops exit only on `None`, and whichever loop sees `None`
                        // clears the gauge — so no wakeup can be dropped and the gauge
                        // still reads "some boundary is parked", which is what the
                        // wedge alert asks.
                        let parked = et.lock().await.pending_boundary();
                        match parked {
                            None => {
                                parked_gauge.set(0);
                                break;
                            }
                            Some(parked_height) => {
                                parked_gauge.set(parked_height as i64);
                                pokes += 1;
                                if pokes.is_multiple_of(PARKED_BOUNDARY_WARN_EVERY) {
                                    warn!(
                                        boundary = parked_height,
                                        pokes,
                                        "epoch boundary still parked awaiting EL state \
                                         materialization; re-poking (no give-up)"
                                    );
                                }
                                ctx_inner
                                    .sleep(
                                        fluentbase_staking_reader::epoch_transition::PENDING_RETRY_BACKOFF,
                                    )
                                    .await;
                            }
                        }
                    }
                })
                .catch_unwind()
                .await;
                if repoke.is_err() {
                    repoke_panics.inc();
                    error!(
                        boundary = number,
                        "epoch-boundary re-poke task PANICKED: this boundary lost its only \
                         driver, so a parked boundary stays parked until some other entry \
                         re-fires it (committee rotation wedges meanwhile). See \
                         boundary_repoke_task_panics + parked_boundary_height."
                    );
                }
            }));
        });

        let boundary_hook: Arc<dyn Fn(crate::order_block::OrderBlock) + Send + Sync> = {
            let enter = enter_boundary.clone();
            Arc::new(move |block: crate::order_block::OrderBlock| enter(block.height))
        };

        // The read-floor seam — the height-floor twin of `enter_boundary`, on the SAME
        // `EpochTransition`. A re-jump landing publishes its result-final floor here and
        // the state machine clamps every later committee read to it, so the entry above
        // reads state this node still holds rather than state the jump left behind.
        // AWAITED by the executor rather than spawned like the entry: the floor has to be
        // in place before the entry's first read, and a spawn would race it.
        let et_for_read_floor = et_arc.clone();
        let read_floor_boundary: crate::executor::BoundaryReadFloorFn =
            Arc::new(move |height: u64| {
                let et = et_for_read_floor.clone();
                Box::pin(async move {
                    et.lock().await.raise_anchor_height(height);
                })
            });

        let me = peer_keypair.public_key();
        info!(peer_pubkey = %me, "DPoS peer identity");

        // The live-DKG `ceremony_store` (written by the always-on `DkgActor`) and the
        // `committee_for` resolver arrive SHARED from the persistent beacon plane
        // (node crate) via [`SharedBeaconPlane`]; this signer engine is a read-only
        // consumer (the `PK_epoch` resolver behind `BeaconVerify`, and the per-epoch
        // signing material via `beacon_resolver`). The disk reload of
        // `<datadir>/beacon/` happened ONCE at the plane's startup — not here.
        //

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

        // Per-epoch threshold beacon resolver for the combined consensus scheme —
        // see `beacon_share_resolver`: carry-forward under the frozen on-chain
        // `dkgQual`-bit arbitration (`beacon::carry`); refusal ⇒ the epoch_manager
        // share-gate demotes to verify-only, the recompute-heal re-promotes.

        // Bulk catch-up committee reader for the EpochManager span soft-enter:
        // load the node's CURRENT finalized tip (re-read every call — a catch-up
        // node's tip advances as the gap closes) and read the contiguous on-chain
        // committee prefix for the requested span via EpochTransition's
        // side-effect-free `soft_enter_span` (committees resolve at the
        // result-final state, anchor − K). Returns `(epoch, snap)` pairs; the
        // consensus side (outer.rs) builds + registers the verify-only scheme.
        // Finalized tip from `canonical_state.get_finalized_num_hash()` — NOT
        // `chain_info().best_number`, which is frozen during pipeline backfill on
        // a deeply-behind node (see MEMORY reth-sync-progress note).
        let et_for_span = et_arc.clone();
        let canonical_for_span = canonical_state.clone();
        let soft_enter_committees: SoftEnterCommittees = Arc::new(move |from: Epoch, to: Epoch| {
            let et = et_for_span.clone();
            let canonical = canonical_for_span.clone();
            Box::pin(async move {
                let anchor = canonical.get_finalized_num_hash().map_or(0, |nh| nh.number);
                let collected = StdMutex::new(Vec::new());
                let record = |epoch: u64, snap: ValidatorSetSnapshot| {
                    collected
                        .lock()
                        .expect("soft-enter span collector")
                        .push((epoch, snap));
                };
                et.lock()
                    .await
                    .soft_enter_span(from.get(), to.get(), anchor, &record)
                    .await;
                collected.into_inner().expect("soft-enter span collector")
            })
        });

        // Steady-state self-healing re-jump (finding #6): the executor's reaction
        // to its own `Update::Tip` event. The cold-start `cold_start_jump` above
        // runs ONCE pre-engine; this closure is its steady-state TWIN — same
        // `upstream` / `RethCommitteeSource` / `RethElSync` / activation, same
        // forward-only BLS-verified `cold_start_jump`, but re-runnable while the
        // executor runs. Enabled wherever an upstream is configured, which since the
        // plane-native default is EVERY launched node — `node/src/dpos.rs:1683` wraps
        // both the `Plane` and the `Ws` branch in `Some(`, so a plain validator has
        // one too and re-jumps plane-natively. The executor runs it synchronously in
        // its `select!` arm, so its `sync_to` FCU is serialized with every other
        // reth write the executor makes — the executor stays the sole reth writer.
        //
        // Rule Y: the validator-with-upstream re-jump is now SYMMETRIC with the
        // follower — it shares the inlet⇄executor `upstream_frontier`, uses the
        // epoch-relative threshold, and wires the same `rotate` escape.
        let re_jump_threshold = crate::cold_start_jump::JUMP_THRESHOLD.min(interval as u64);
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let up = up.clone();
            // The inlet's SAME upstream-rotation escape (Rule L/Y); bound BEFORE the
            // cb moves `up`.
            let rotate = up.rotate_callback();
            // Erased `get_latest`-height probe for the executor's frozen-tip
            // frontier prod (see `executor::ReJump::probe`); bound BEFORE the cb
            // moves `up`.
            let frontier_probe: crate::executor::FrontierProbeFn = {
                let up = up.clone();
                Arc::new(move || {
                    let up = up.clone();
                    Box::pin(async move {
                        crate::cert_follow::CertUpstream::get_latest(&up)
                            .await
                            .map(|uf| Height::new(uf.block.height))
                    })
                })
            };
            let provider = provider.clone();
            let evm_config = evm_config.clone();
            let staking_config = staking_config.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let ctx = ctx.clone();
            let peer_count = peer_count.clone();
            let cb: crate::executor::ReJumpFn = Arc::new(move |from: u64| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let beacon_engine_handle = beacon_engine_handle.clone();
                let peer_count = peer_count.clone();
                // `verify_jump_authenticated` needs a `&mut (Clock + CryptoRngCore)`;
                // a fresh clone per call so the closure stays re-usable.
                let mut jump_ctx = ctx.clone();
                Box::pin(async move {
                    let committees = crate::cert_inlet::RethCommitteeSource::new(
                        RethStakingStateReader::new(provider.clone(), evm_config, staking_config),
                        chain_id,
                        {
                            let p = provider.clone();
                            Arc::new(move || {
                                let n = p.finalized_block_number().ok()??;
                                p.block_hash(n).ok().flatten()
                            })
                        },
                    );
                    let el = crate::cold_start_jump::RethElSync::new(
                        jump_ctx.clone(),
                        provider.clone(),
                        beacon_engine_handle,
                        dpos_activation_block,
                        peer_count,
                    );
                    // Return the typed terminal `JumpOutcome` verbatim — the
                    // executor's completion arm classifies it (Landed re-seeds;
                    // Stalled is NON-fatal + retried on the next Tip; AuthFailed
                    // is fail-closed). §9.6.
                    crate::cold_start_jump::cold_start_jump_with_threshold(
                        from,
                        &up,
                        &committees,
                        &el,
                        // No L1 checkpoint on the validator path (trustless
                        // POST-sync committee read at the landing).
                        None,
                        dpos_activation_block,
                        re_jump_threshold,
                        &mut jump_ctx,
                    )
                    .await
                }) as futures::future::BoxFuture<'static, _>
            });
            crate::executor::ReJump {
                call: cb,
                // Rule Y: SHARED inlet⇄executor frontier — the validator inlet (when
                // this node is an inlet-fed joiner) advances it on every cert, so a
                // frozen marshal tip can't mask a deep gap, exactly as on the follower.
                upstream_frontier: upstream_frontier.clone(),
                // Epoch-relative gate, mirroring the follower (real-prod epochs ≫ 1024
                // keep the serving-window size; a compressed test epoch heals within
                // an epoch).
                threshold: re_jump_threshold,
                // Rule L/Y: the same upstream-rotation escape the follower wires (T2 —
                // safe now that BadTarget is NON-fatal).
                rotate: Some(rotate),
                // Frozen-tip frontier probe — the live-follow driver for the
                // PLANE-NATIVE validator (no inlet writes `upstream_frontier`, no
                // consensus participation while rotated out): the executor probes
                // `get_latest` when its tip freezes and hints the marshal toward
                // the discovered frontier. Also a harmless backstop on the WS path
                // (the inlet keeps the tip advancing → the probe stays silent).
                probe: Some(frontier_probe),
            }
        });

        // Boundary seeding seam — the same upstream and the same POST-sync committee
        // read as the re-jump above, but fetching ONE already-finalized block by
        // height rather than a landing. After any jump the epoch-terminal height that
        // `Inline::genesis` (and so the engine-spawn gate) needs sits BELOW the new
        // marshal floor, where no repair path ever fetches it — so a member that did
        // not already hold it parks verify-only for the whole landing epoch, neither
        // proposing nor voting.
        let boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn> =
            upstream.as_ref().map(|up| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let ctx = ctx.clone();
                let sync_metrics = sync_metrics.clone();
                Arc::new(move |height: u64, at_hash: B256| {
                    let up = up.clone();
                    let provider = provider.clone();
                    let evm_config = evm_config.clone();
                    let staking_config = staking_config.clone();
                    let sync_metrics = sync_metrics.clone();
                    // `verify_jump_authenticated` needs a `&mut (Clock + CryptoRngCore)`;
                    // a fresh clone per call so the closure stays re-usable.
                    let mut fetch_ctx = ctx.clone();
                    Box::pin(async move {
                        let committees = crate::cert_inlet::RethCommitteeSource::new(
                            RethStakingStateReader::new(
                                provider.clone(),
                                evm_config,
                                staking_config,
                            ),
                            chain_id,
                            {
                                let p = provider.clone();
                                Arc::new(move || {
                                    let n = p.finalized_block_number().ok()??;
                                    p.block_hash(n).ok().flatten()
                                })
                            },
                        );
                        crate::cert_follow::fetch_verified_boundary(
                            &up,
                            &committees,
                            &mut fetch_ctx,
                            &sync_metrics,
                            at_hash,
                            height,
                        )
                        .await
                    }) as futures::future::BoxFuture<'static, _>
                }) as crate::cert_follow::BoundaryFetchFn
            });

        // Core- and executor-owned families, registered against the launch context
        // — the SAME context `BeaconMetrics` is registered against inside
        // `beacon::build`, because commonware prefixes each family with the
        // context's label path and a labelled child would silently rename them.
        // Registered on this path AND on the follower's, since both node classes
        // reach the epoch manager and the executor.
        let epoch_metrics = crate::epoch_manager::EpochEngineMetrics::default();
        epoch_metrics.register(&ctx);
        let executor_metrics = crate::executor::ExecutorMetrics::default();
        executor_metrics.register(&ctx);

        let outer = OuterBuilder {
            me: me.clone(),
            // The beacon plane's epoch-key agreement instances, adopted by the
            // epoch manager so they prune on the engine cutoff.
            agreement_intake,
            // Bug A: no-op the consensus vote/cert-plane blocker (parity with the
            // beacon-resolver NoopBlocker, review [1013]). The simplex batcher's
            // evidence-free `block!(self.blocker, signer, ..)` on a transient
            // batch-verify-failed vote would otherwise 4h-partition an honest
            // committee peer on the shared global transport. Slashing is preserved:
            // equivocation evidence rides `Activity::Conflicting*` independently of
            // `block!`. `provider:` keeps the real oracle for peer-set tracking.
            blocker: NoopBlocker,
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block,
            signer_keypair: Some(bls_keypair),
            randomness: randomness.clone(),
            spawn_unblocked,
            re_jump,
            soft_enter_committees,
            epoch_metrics: epoch_metrics.clone(),
            executor_metrics: executor_metrics.clone(),
            sync_metrics: sync_metrics.clone(),
            safety_halt: safety_halt.clone(),
            tombstones,
            plane_clock,
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
            // Fresh migration: anchor = activation. Restart: a raises-only no-op
            // (the archive's floor is already at/above its own finalized). After
            // a cold-start JUMP the floor is `landing − K` (the K below-landing
            // blocks are derivable via the inlet's ordinary pulls + the executor
            // gap-walk).
            marshal_floor: Some(
                jumped_marshal_floor.unwrap_or_else(|| Height::new(latest_finalized)),
            ),
            boundary_fetch,
            boundary_enter: enter_boundary,
            boundary_read_floor: read_floor_boundary,
            fcu_heartbeat_interval: Duration::from_secs(8),
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_address,
            slasher_reader: reader_for_slasher,
            slasher_latest_finalized_hash,
            slasher_sink,
            slasher_wal_partition: "slasher-wal".into(),
            slasher_evidence: Some(evidence),

            feed,

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
        }
        // `seed_journal_writer` is cloned off `ctx`, NOT off the `outer_engine`
        // context: it must be a sibling of the engine task so `engine.abort()`
        // does not cascade into the writer before it has drained (see
        // `OuterBuilder::build`).
        .build(ctx.with_label("outer_engine"))
        .await?;

        // Register the initial epoch's BlsScheme so marshal can verify
        // certificates from this epoch before any boundary fires.
        let namespace = fluent_namespace(chain_id);
        let initial_committee = epoch_committee_from_snapshot(&initial_snapshot)
            .map_err(|e| eyre!("initial snapshot has non-unique participants: {e:?}"))?;
        // Cold-start register: the marshal is empty at launch, so no beacon key
        // is resolvable → `cert_seed_pin = None` ⇒ vote-only cert verify (the
        // accepted residual window, upgraded once boundary blocks flow through the
        // inlet cursor / soft-enter walk — bug 2 sign-off item 1).
        let initial_scheme = build_verifier(&namespace, initial_committee.bimap, None, None);
        outer.cold_start_register(Epoch::new(initial_epoch_u64), initial_scheme);

        // Bridge forwarder: drains (u64, snap) queued by EpochTransition
        // and converts to (Epoch, snap) for OuterEngine's boundary receiver.
        let outer_boundary_tx = outer.boundary_sender();
        let shutdown_for_forwarder = shutdown.clone();
        // SUPERVISED, not detached: the `shutdown.cancel()` below is the ERROR
        // path only — a panic inside this loop unwinds straight past it, and
        // under `with_catch_panics(true)` that panic is one log line and nothing
        // else. Dropping the handle would detach the task (no `Drop` on
        // commonware handles), so committee rotation would stop while the node
        // stayed "healthy". Handed to the host supervisor instead, which treats
        // ANY resolution (panic → `Err(Error::Exited)`, or the clean-exit warn)
        // as fatal — so the deliberate fail-fast holds for both paths.
        let epoch_bridge_handle = ctx.with_label("epoch_bridge").spawn(move |_| async move {
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
        });

        // Grab the marshal mailbox for the node-side cert feed/RPC BEFORE
        // `start` consumes the engine.
        let cert_mailbox = outer.marshal_mailbox();

        // Start OuterEngine — ctx + the 5 plane-owned non-beacon `MuxHandle`s
        // (vote/cert/resolver per-epoch register, broadcast/marshal subchannel 0) +
        // this promotion's fresh vote-backup receiver. The OuterEngine registers its
        // sub-channels in `run`; on demote it drops them (auto-deregister) — the
        // plane's broker tasks stay live for the next promotion.
        //
        // `upstream` (the `--dpos.follower-upstream` handle, also used by the
        // cold-start jump above by reference) is threaded into the marshal's
        // by-height backfill resolver: when `Some`, the marshal pulls the cold-start
        // `[floor+1 .. first_live]` gap from the UPSTREAM (not consensus-plane peers),
        // so an OUT-OF-COMMITTEE validator (a not-yet-committee external joiner with
        // zero consensus-plane connectivity) backfills exactly like a follower
        // instead of wedging. The resolver keeps its own `upstream` clone alive
        // (which keeps the WS actor alive). A no-upstream validator passes `None` and
        // keeps the consensus-plane p2p resolver (the treadmill, unchanged). The
        // marshal still BLS-verifies every delivered cert — trustless, single-writer
        // intact.
        let consensus_handle = outer.start(
            ctx.with_label("marshal_resolver"),
            vote_mux,
            cert_mux,
            resolver_mux,
            broadcast_mux,
            marshal_mux,
            vote_backup_rx,
            upstream,
        );

        Ok(DposLayerHandle {
            consensus_handle,
            cert_mailbox,
            supervised: vec![("epoch_bridge", epoch_bridge_handle)],
            // Both journal writers moved into `beacon::build` with the stores
            // they back, so this layer has no drain of its own left. The node
            // collects them off the `Beacon` instead — see its `shutdown`.
            drain_on_shutdown: vec![],
            // The validator serves from its beacon plane's own
            // `Beacon::artifact_bytes`, which the node holds directly.
            artifact_bytes: None,
        })
    }
}

/// Reth handles a near-planeless FOLLOWER needs (Phase 3). Distinct from
/// [`RethHandle`] only in that a follower carries NO slasher/pool transport (it
/// never signs) — the slasher actor is built (cheap) but never started.
pub struct FollowerRethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    pub canonical_state: reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    pub genesis_hash: B256,
    /// Read-only probe of reth's connected devp2p peer count — see
    /// [`RethHandle::peer_count`]. Drives the follower EL-sync no-peers net.
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
}

/// Operator-supplied config for the near-planeless follower (Phase 3). The node
/// owns the WS upstream + the ONE broadcast `Muxer`; this config carries the
/// reth-evm collaborators + the cert-inlet feed channels.
pub struct FollowerLayerConfig<D, XC, A, U> {
    /// This node's ed25519 peer identity (the FluentP2P crypto's public key),
    /// threaded from the node. A standalone follower never gossips, but
    /// `buffered::Engine` + the marshal resolver are keyed on it.
    pub me: commonware_cryptography::ed25519::PublicKey,
    pub staking_config: StakingReaderConfig,
    /// Datadir path of the fork-safety halt marker
    /// ([`crate::sync_metrics::SafetyHalt::restoring`]). A marker left by a
    /// previous run brings this node up permanently verify-only — the latch has
    /// no in-process `disengage`, so without it a restart silently cleared a
    /// halt and the node signed again on the same disk. `None` only in tests.
    pub halt_marker: Option<std::path::PathBuf>,
    /// L1 Rollup-checkpoint hash (B2). `Some` ⇒ fail-closed post-EL-sync assert
    /// (`cert-follow: L1 Rollup checkpoint …`); `None` ⇒ the upstream head is the
    /// only trust input (devnet fallback).
    pub l1_checkpoint_hash: Option<B256>,
    /// OrderBlock → derived-EVM-block execution (node-built over reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-built over the reth provider).
    pub executed: XC,
    /// Pool-backed ordering assembly (node-built — pool trait bounds live there).
    /// A follower never proposes, so this is never exercised; the OuterBuilder
    /// requires it at the type level.
    pub assembler: Arc<A>,
    pub fee_recipient: Address,
    pub target_gas_limit: u64,
    /// Cert-feed sink (the marshal's 2nd application-`Reporter`) for this node's
    /// `consensus` RPC latest-tier. `None` for nodes that don't serve the feed.
    pub feed: Option<crate::feed_sink::FeedSink>,
    pub fcu_heartbeat_interval: Duration,
    /// Cert upstream for the single-shot, pre-engine cold-start EL-sync JUMP
    /// (`get_latest`). A follower ALWAYS has an upstream (the WS the inlet uses);
    /// `None` only in tests.
    pub upstream: Option<U>,
    /// The live finalized-cert stream the node's WS actor pushes — the inlet's
    /// SOLE producer (a follower forms no certs locally).
    pub finalized_rx: mpsc::Receiver<crate::cert_follow::UpstreamFinalized>,
    /// Connection-generation token the node's WS actor bumps on each (re)connect
    /// (#7). Wired into the inlet via `with_connection_token` so the data-fault
    /// streak is scoped to the LIVE connection — a connection-level auto-rotation
    /// (which the inlet cannot otherwise observe) resets the streak, so one
    /// upstream's faults never bleed into the next URL's rotation budget. `None`
    /// in tests.
    pub conn_gen: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// B3 — the serving-window sink: each VERIFIED inlet pair is forwarded here
    /// so a tier-2 follower aligns via THIS node's `consensus` WS window. `None`
    /// = no serving (tests).
    pub verified_tx:
        Option<tokio::sync::mpsc::UnboundedSender<crate::cert_follow::UpstreamFinalized>>,
}

/// A no-op [`SlasherTxSink`] for a follower: the slasher actor is built (cheap)
/// but never started, so `submit` is unreachable — a non-signer follower can
/// never detect-and-submit equivocation. Avoids dragging the node's
/// signer/pool `PoolTxSink` onto a keyless follower.
struct NoopSlasherSink;

impl SlasherTxSink for NoopSlasherSink {
    fn submit<'a>(
        &'a self,
        _target: Address,
        _calldata: alloy_primitives::Bytes,
    ) -> std::pin::Pin<
        Box<dyn core::future::Future<Output = crate::slasher::actor::SubmitOutcome> + Send + 'a>,
    > {
        Box::pin(async {
            crate::slasher::actor::SubmitOutcome::Failed(
                "follower NoopSlasherSink: a non-signer never submits slashing".into(),
            )
        })
    }
}

impl DposLayer {
    /// Launch the near-planeless FOLLOWER: an OuterEngine with
    /// `signer_keypair: None` driven by the cert-inlet (the only producer for a
    /// non-validator) instead of a local BFT engine. The executor is the SOLE reth
    /// writer (plus the single-shot pre-engine `cold_start_jump`).
    ///
    /// Steps:
    ///   1. resolve geometry (RESTART vs FRESH datadir via `read_geometry`) +
    ///      EL-sync onto the upstream's attested tip,
    ///   2. B2 — assert the L1 Rollup checkpoint (verbatim strings) post-EL-sync,
    ///   3. `cold_start_jump` for a deep residual gap (forward-only, BLS-verified),
    ///   4. build the follower OuterEngine (`signer_keypair: None`, no beacon
    ///      plane/signer/slasher-start, `Option<Muxes>::None`),
    ///   5. `start_follower` over the ONE broadcast `Muxer`, with an
    ///      UPSTREAM-backed marshal resolver (`UpstreamResolver`) that backfills
    ///      the by-height floor→frontier gap the live stream never carries,
    ///   6. spawn the cert-inlet over `finalized_rx` → `marshal_mailbox()` (+ the
    ///      B3 serving window).
    #[allow(clippy::too_many_arguments)]
    pub async fn launch_follower<Provider, EvmConfig, BeaconEngine, D, XC, A, U>(
        ctx: Context,
        reth: FollowerRethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: FollowerLayerConfig<D, XC, A, U>,
        oracle: fluentbase_p2p::OracleHandle,
        broadcast_mux: PlaneMux,
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
        U: crate::cert_follow::CertUpstream,
    {
        let FollowerRethHandle {
            provider,
            evm_config,
            beacon_engine_handle,
            chain_id,
            canonical_state,
            genesis_hash,
            peer_count,
        } = reth;
        let FollowerLayerConfig {
            me,
            staking_config,
            halt_marker,
            l1_checkpoint_hash,
            deriver,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            feed,
            fcu_heartbeat_interval,
            upstream,
            finalized_rx,
            conn_gen,
            verified_tx,
        } = cfg;

        // Self-heal stuck-detector: registered ONCE for the follower launch (the
        // follower has no beacon plane, so it owns its own — mirrors the fresh
        // `BeaconMetrics` below). Threaded into the cold-start jump / activation-wait
        // / landing-belt self-heal loops.
        let sync_metrics = SyncMetrics::default();
        sync_metrics.register(&ctx);
        // Fork-safety latch (Phase 3): a follower's executor derives+imports off the
        // inlet and can hit result divergence / EL Invalid, so it too must be able to
        // halt-and-stay-up rather than crash.
        let safety_halt = match halt_marker {
            Some(path) => crate::sync_metrics::SafetyHalt::restoring(sync_metrics.clone(), path),
            None => crate::sync_metrics::SafetyHalt::new(sync_metrics.clone()),
        };

        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        // Epoch geometry. RESTART datadir: `ChainConfig` readable from local
        // state. FRESH datadir (runtime-deployed cluster): geometry unreadable
        // locally → EL-sync FIRST, then read it at the synced landing. `read_geometry`
        // discriminates (it returns `None` when `ChainConfig`/activation are absent).
        let (_rf_num, rf_hash, _h0_num, _h0_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);
        let mk_el_sync = |activation: u64| {
            crate::cold_start_jump::RethElSync::new(
                ctx.clone(),
                provider.clone(),
                beacon_engine_handle.clone(),
                activation,
                peer_count.clone(),
            )
        };

        let (activation, interval, mut anchor_height, mut anchor_hash) =
            match read_geometry(&reader, rf_hash)? {
                Some((activation, interval)) => {
                    let el = mk_el_sync(activation);
                    let (h, hash) = match upstream.as_ref() {
                        Some(up) => match up.get_latest().await {
                            Some(latest) => el.sync_to(&latest).await?,
                            None => {
                                warn!(
                                    "cert-follow: upstream getLatest returned none; relying on \
                                     existing reth state"
                                );
                                let hash = wait_for_activation_block(
                                    &ctx,
                                    &provider,
                                    activation,
                                    &sync_metrics,
                                )
                                .await?;
                                (activation, hash)
                            }
                        },
                        None => {
                            let hash = wait_for_activation_block(
                                &ctx,
                                &provider,
                                activation,
                                &sync_metrics,
                            )
                            .await?;
                            (activation, hash)
                        }
                    };
                    (activation, interval, h, hash)
                }
                None => {
                    // Fresh datadir: sync without an activation clamp (unknown
                    // yet), then read geometry at the landing and re-clamp.
                    let up = upstream.as_ref().ok_or_else(|| {
                        eyre!(
                            "cert-follow: fresh datadir without a local ChainConfig needs a \
                             reachable upstream to EL-sync from"
                        )
                    })?;
                    let latest = up.get_latest().await.ok_or_else(|| {
                        eyre!(
                            "cert-follow: fresh datadir without a local ChainConfig needs a \
                             reachable upstream to EL-sync from"
                        )
                    })?;
                    let (h, hash) = mk_el_sync(0).sync_to(&latest).await?;
                    let (activation, interval) =
                        read_geometry(&reader, hash)?.ok_or_else(|| {
                            eyre!(
                                "cert-follow: ChainConfig still not deployed at the synced tip \
                                 {h} — wrong chain, or the upstream predates DPoS activation"
                            )
                        })?;
                    let h = h.max(activation);
                    let hash = read_with_visibility_belt(
                        &ctx,
                        &sync_metrics,
                        &format!("the clamped landing {h}"),
                        || {
                            provider
                                .block_hash(h)
                                .wrap_err("reading clamped landing hash")
                        },
                    )
                    .await?;
                    (activation, interval, h, hash)
                }
            };

        // B2 — L1 Rollup-checkpoint assert (verbatim strings), post-EL-sync,
        // fail-closed: a bogus-checkpoint cert-cascade follower still errors with
        // "is NOT in the local chain after EL-sync".
        if let Some(l1_hash) = l1_checkpoint_hash {
            crate::cold_start_jump::assert_l1_checkpoint(&provider, l1_hash)?;
        }

        // Deep catch-up: a residual gap above JUMP_THRESHOLD re-runs the EL-sync
        // phase and re-seeds the anchor + marshal floor at landing − K. Forward-only,
        // BLS-verified (or L1-gated). Runs BEFORE the OuterEngine/executor — the same
        // single-writer mutual-exclusion `recover_finalized_tail_into_reth` relies on.
        let mut jumped_marshal_floor: Option<Height> = None;
        if let Some(up) = upstream.as_ref() {
            let committees = crate::cert_inlet::RethCommitteeSource::new(
                RethStakingStateReader::new(
                    provider.clone(),
                    evm_config.clone(),
                    staking_config.clone(),
                ),
                chain_id,
                {
                    let p = provider.clone();
                    Arc::new(move || {
                        let n = p.finalized_block_number().ok()??;
                        p.block_hash(n).ok().flatten()
                    })
                },
            );
            let el = mk_el_sync(activation);
            let mut jump_ctx = ctx.clone();
            if let Some((h, hash, floor)) = cold_start_jump_self_heal(
                &ctx,
                &sync_metrics,
                anchor_height,
                up,
                &committees,
                &el,
                l1_checkpoint_hash,
                activation,
                &mut jump_ctx,
            )
            .await?
            {
                anchor_height = h;
                anchor_hash = hash;
                jumped_marshal_floor = Some(Height::new(floor));
            }
        }

        // Two-tier seed: the landing block's own result attestation arrives only
        // K blocks later, so the finalized tier starts at landing − K (clamped to
        // activation). Seed reth's FCU at the floor so it never finalizes ahead of
        // the result tier.
        let finalized_floor = anchor_height.saturating_sub(K).max(activation);
        let finalized_hash = read_with_visibility_belt(
            &ctx,
            &sync_metrics,
            &format!("the finality floor {finalized_floor}"),
            || {
                provider
                    .block_hash(finalized_floor)
                    .wrap_err("reading finality floor hash")
            },
        )
        .await?;
        let _ = beacon_engine_handle
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: anchor_hash,
                safe_block_hash: finalized_hash,
                finalized_block_hash: finalized_hash,
            })
            .await;

        let initial_epoch_u64 =
            fluentbase_staking_reader::reader::epoch_of_block(anchor_height, interval, activation);
        info!(
            chain_id,
            activation,
            interval,
            anchor_height,
            initial_epoch = initial_epoch_u64,
            "cert-follow (inlet) cold-start resolved"
        );

        // Consensus genesis = the ordering-chain anchor artifact at the resumed
        // height (every node derives the identical artifact; Simplex
        // `set_genesis(digest)` matches view 1's parent). A follower never
        // proposes, but the marshal/executor still need the genesis anchor.
        let genesis_unsealed = read_with_visibility_belt(
            &ctx,
            &sync_metrics,
            &format!("follower genesis block at height {anchor_height}"),
            || {
                provider
                    .block_by_number(anchor_height)
                    .wrap_err("reading follower genesis block")
            },
        )
        .await?;
        let genesis_sealed: SealedBlock<RethBlock> = SealedBlock::seal_slow(genesis_unsealed);
        let genesis_block = anchor_order_block(&genesis_sealed)?;

        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head block number at startup")?;
        let head_info = canonical_state.chain_info();

        let epoch_length_blocks =
            NonZeroU64::new(interval as u64).ok_or_eyre("epoch_block_interval must be > 0")?;
        // The beacon-owned families are registered by `beacon::for_follower`
        // below, which is this path's randomness provider — NOT here. A standalone
        // `BeaconMetrics::register` next to it would register every one of them
        // TWICE: `prometheus_client::Registry::register` neither deduplicates nor
        // complains, so the only symptom is a duplicated family at scrape time.
        //
        // Registered here, on the SAME context, for the reason the metric-owner
        // split exists: these families are core- and executor-owned on BOTH node
        // classes, and `prometheus_client::Registry::register` neither
        // deduplicates nor complains — a family with no owner on this path would
        // simply stop appearing in follower scrapes.
        let epoch_metrics = crate::epoch_manager::EpochEngineMetrics::default();
        epoch_metrics.register(&ctx);
        let executor_metrics = crate::executor::ExecutorMetrics::default();
        executor_metrics.register(&ctx);

        // Slasher reader + fallback are required by the OuterBuilder type but the
        // slasher is never STARTED on a follower (`run_follower` drops the unstarted
        // actor) — a non-signer never submits slashing.
        let slasher_reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );
        let provider_for_finalized = provider.clone();
        let slasher_latest_finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync> =
            Arc::new(move || {
                let n = provider_for_finalized.finalized_block_number().ok()??;
                provider_for_finalized.block_hash(n).ok().flatten()
            });

        // Bulk catch-up span reader: a follower soft-enters every live epoch but
        // never spawns an engine, so this is still used to register verify-only
        // schemes for the marshal across a gap. Read committees at the current
        // finalized tip via a per-call EpochTransition-free reader.
        let canonical_for_span = canonical_state.clone();
        let provider_for_span = provider.clone();
        let evm_for_span = evm_config.clone();
        let staking_for_span = staking_config.clone();
        let soft_enter_committees: SoftEnterCommittees = Arc::new(move |from: Epoch, to: Epoch| {
            let canonical = canonical_for_span.clone();
            let reader = RethStakingStateReader::new(
                provider_for_span.clone(),
                evm_for_span.clone(),
                staking_for_span.clone(),
            );
            Box::pin(async move {
                // Read each committee at the current finalized hash. That is safe on
                // TWO counts and only the first was true before 2026-07-31: the
                // committee array and the one-shot keys are frozen storage, and the
                // per-member leader weight is now frozen too (`leaderStakes[epoch]`,
                // stamped at commit). This path would survive either way — it feeds
                // verify-only scheme registration and reads keys, never stakes — but
                // the blanket "content-invariant" claim it used to make was false for
                // the stakes leg, and that is exactly the claim a future reader would
                // trust on a path where it DOES matter. Truncate at the first
                // missed/unreadable committee → a contiguous prefix.
                let at_hash = canonical
                    .get_finalized_num_hash()
                    .map(|nh| nh.hash)
                    .unwrap_or_default();
                let mut out = Vec::new();
                for e in from.get()..=to.get() {
                    match reader.epoch_committee_snapshot(e, at_hash) {
                        Ok(snap) if !snap.validators.is_empty() => out.push((e, snap)),
                        _ => break,
                    }
                }
                out
            })
        });

        // Steady-state self-healing re-jump (finding #6): the follower's executor
        // reaction to its own `Update::Tip` event — the steady-state twin of the
        // pre-engine `cold_start_jump` above. Same upstream / committee source /
        // EL-sync / activation / L1 checkpoint. A follower ALWAYS has an upstream
        // (the WS the inlet uses), so this is set whenever `upstream.is_some()`.
        //
        // The TRUE upstream frontier: the cert-inlet advances it on every received
        // cert (INCLUDING the committee-not-committed deferred ones), and the
        // executor's re-jump trigger reads it — so a follower whose marshal tip
        // FROZE under the defer deadlock still sees the climbing frontier and
        // re-jumps out. ONE atomic shared between the inlet (writer) and the
        // re-jump trigger (reader). See [`crate::executor::ReJump::upstream_frontier`].
        let upstream_frontier = Arc::new(std::sync::atomic::AtomicU64::new(0));
        // Epoch-relative re-jump gate: the defer deadlock is "≥2 epochs behind", so
        // recovery fires at `min(serving-window, 1 epoch)` — real-prod epochs ≫ 1024
        // keep 1024; a compressed test epoch heals within an epoch (see `ReJump::threshold`).
        let re_jump_threshold = crate::cold_start_jump::JUMP_THRESHOLD.min(interval as u64);
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let up = up.clone();
            // The inlet's SAME upstream-rotation escape (Rule L): the re-jump's
            // terminal fault (`BadTarget` / repeated `Stalled`) rotates the SAME WS
            // actor mailbox the inlet's `inlet_rotate` wraps → coalesced at the WS
            // actor. Bound BEFORE the cb moves `up`.
            let rotate = up.rotate_callback();
            let provider = provider.clone();
            let evm_config = evm_config.clone();
            let staking_config = staking_config.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let ctx = ctx.clone();
            let peer_count = peer_count.clone();
            let cb: crate::executor::ReJumpFn = Arc::new(move |from: u64| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let beacon_engine_handle = beacon_engine_handle.clone();
                let peer_count = peer_count.clone();
                let mut jump_ctx = ctx.clone();
                Box::pin(async move {
                    let committees = crate::cert_inlet::RethCommitteeSource::new(
                        RethStakingStateReader::new(provider.clone(), evm_config, staking_config),
                        chain_id,
                        {
                            let p = provider.clone();
                            Arc::new(move || {
                                let n = p.finalized_block_number().ok()??;
                                p.block_hash(n).ok().flatten()
                            })
                        },
                    );
                    let el = crate::cold_start_jump::RethElSync::new(
                        jump_ctx.clone(),
                        provider.clone(),
                        beacon_engine_handle,
                        activation,
                        peer_count,
                    );
                    // Return the typed terminal `JumpOutcome` verbatim — the
                    // executor's completion arm classifies it (§9.6).
                    crate::cold_start_jump::cold_start_jump_with_threshold(
                        from,
                        &up,
                        &committees,
                        &el,
                        l1_checkpoint_hash,
                        activation,
                        re_jump_threshold,
                        &mut jump_ctx,
                    )
                    .await
                }) as futures::future::BoxFuture<'static, _>
            });
            crate::executor::ReJump {
                call: cb,
                upstream_frontier: upstream_frontier.clone(),
                threshold: re_jump_threshold,
                rotate: Some(rotate),
                // No probe on the follower: its WS inlet is the ALWAYS-ON live
                // producer (subscribe + gap-repair + conn-generation rotation), so
                // the frozen-tip prod has nothing to add — the probe exists for the
                // inlet-less plane-native validator.
                probe: None,
            }
        });

        // Boundary seeding seam — see the twin in `launch` for the full rationale. A
        // follower spawns no per-epoch engine, so the `Inline::genesis` precondition
        // this seeds is not its own concern; it is wired here because a follower runs
        // the SAME jump and therefore leaves the SAME below-floor hole, and an
        // upstream-configured node can be promoted to a seated validator by a later
        // committee change without restarting.
        let boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn> =
            upstream.as_ref().map(|up| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let ctx = ctx.clone();
                let sync_metrics = sync_metrics.clone();
                Arc::new(move |height: u64, at_hash: B256| {
                    let up = up.clone();
                    let provider = provider.clone();
                    let evm_config = evm_config.clone();
                    let staking_config = staking_config.clone();
                    let sync_metrics = sync_metrics.clone();
                    let mut fetch_ctx = ctx.clone();
                    Box::pin(async move {
                        let committees = crate::cert_inlet::RethCommitteeSource::new(
                            RethStakingStateReader::new(
                                provider.clone(),
                                evm_config,
                                staking_config,
                            ),
                            chain_id,
                            {
                                let p = provider.clone();
                                Arc::new(move || {
                                    let n = p.finalized_block_number().ok()??;
                                    p.block_hash(n).ok().flatten()
                                })
                            },
                        );
                        crate::cert_follow::fetch_verified_boundary(
                            &up,
                            &committees,
                            &mut fetch_ctx,
                            &sync_metrics,
                            at_hash,
                            height,
                        )
                        .await
                    }) as futures::future::BoxFuture<'static, _>
                }) as crate::cert_follow::BoundaryFetchFn
            });

        // The follower's boundary trigger, producer half. `boundary_hook` fires
        // synchronously inside the marshal's reporter for every finalized
        // `OrderBlock`, so it may only stamp and wake — the committee read and the
        // delivery run on the driver task below. `fetch_max` + a permit-storing
        // `Notify` is lossless in the only sense that matters here: the driver
        // reads the LATEST height, and a wake landing while it is not armed is held
        // as a permit rather than dropped.
        let follower_finalized = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let follower_finalized_wake = Arc::new(tokio::sync::Notify::new());
        let boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync> = {
            let height = follower_finalized.clone();
            let wake = follower_finalized_wake.clone();
            Arc::new(move |block: OrderBlock| {
                height.fetch_max(block.height, std::sync::atomic::Ordering::Relaxed);
                wake.notify_one();
            })
        };

        // FROZEN on-chain `dkgQual[e]` reader — which epoch MINTED the key in
        // force at a given epoch. A stable committee runs no agreement at all, so
        // an artifact for the epoch being verified does not exist; without this
        // the key-delivery rung below would ask for one that was never minted and
        // miss forever. The read itself needs nothing from the beacon plane a
        // follower does not run: the same reth reader and the same finalized
        // anchor its committee reads already use. The freeze/memo rule lives in
        // `beacon::carry::frozen_dkg_qual` so this copy cannot drift from the
        // validator's.
        let follower_dkg_qual = {
            let reader = RethStakingStateReader::new(
                provider.clone(),
                evm_config.clone(),
                staking_config.clone(),
            );
            let provider = provider.clone();
            crate::beacon::carry::frozen_dkg_qual(
                Arc::new(move || {
                    let fin = provider.finalized_block_number().ok().flatten()?;
                    provider.block_hash(fin).ok().flatten()
                }),
                Arc::new(move |epoch, at| {
                    let bit = reader.dkg_qual(epoch, at).ok()?;
                    // A SET bit is proof the commit happened, so the committee
                    // read is skipped for it.
                    let committed = bit
                        || reader
                            .epoch_committee_snapshot(epoch, at)
                            .map(|s| !s.validators.is_empty())
                            .unwrap_or(false);
                    Some((bit, committed))
                }),
            )
        };
        // `committee[epoch]` with its BLS half — the SOLE authority a fetched
        // artifact is checked against, read from THIS node's own chain state at
        // the finalized anchor. A follower trusts its upstream for DELIVERY and
        // for nothing else: an artifact that does not carry a quorum of this
        // committee is rejected here exactly as a peer's would be on the plane.
        let follower_committee_source: crate::beacon::artifact::CommitteeSource = {
            let canonical = canonical_state.clone();
            let reader = RethStakingStateReader::new(
                provider.clone(),
                evm_config.clone(),
                staking_config.clone(),
            );
            Arc::new(move |epoch: u64| {
                let at = canonical.get_finalized_num_hash()?.hash;
                let snap = reader.epoch_committee_snapshot(epoch, at).ok()?;
                if snap.validators.is_empty() {
                    return None;
                }
                crate::scheme::epoch_committee_from_snapshot(&snap).ok()
            })
        };
        // The ONE relationship a follower has. `None` (a test with no upstream)
        // leaves the rung permanently empty, which is exactly the pre-FLU-1167
        // behaviour: vote-only admission, never a fault.
        let artifact_fetch: crate::beacon::ArtifactFetch = match &upstream {
            Some(up) => {
                let up = up.clone();
                Arc::new(move |epoch: u64| {
                    let up = up.clone();
                    Box::pin(async move {
                        crate::cert_follow::CertUpstream::get_epoch_artifact(&up, epoch).await
                    }) as futures::future::BoxFuture<'static, _>
                })
            }
            None => Arc::new(|_| Box::pin(async { None })),
        };
        // A follower's randomness is negative on everything it PRODUCES — no DKG,
        // no ceremony store, no agreement instance, no share, no seed — and real
        // on the one thing it VERIFIES: the key a certificate of an epoch must be
        // checked against. It used to be `beacon::absent`, i.e. negative there
        // too, BY TYPE and for the life of the process; that was the whole of
        // FLU-1167. What was missing was never the check — `verify_artifact` needs
        // only the chain id, an rng and a `committee[epoch]` read the inlet makes
        // per certificate — but a DELIVERY ROUTE, which the three capabilities
        // above now supply over the cert upstream.
        //
        // The durable key journal that used to be opened here (with an empty
        // partition, i.e. RAM-only) stays DELETED: the store this fills is RAM-only
        // by design, and what a restart loses is one fetch per epoch over a link
        // the follower holds open anyway. Its seed journal goes with it, and that
        // one has a real consequence worth stating — a node that ran as a validator
        // and is restarted as `--cert-follow` no longer replays its seed-journal
        // tail. Nothing on the follower path reads a seed it did not just receive,
        // so the tail was already unreadable there; what is lost is the ability to
        // hand it back on a later switch BACK to validator, which is a restart
        // through this path either way.
        let crate::beacon::FollowerBeacon {
            randomness,
            artifact_bytes,
            fetch_handle: artifact_fetch_handle,
        } = crate::beacon::for_follower(
            &ctx,
            crate::beacon::FollowerRandomnessConfig {
                chain_id,
                committees: follower_committee_source,
                dkg_qual: follower_dkg_qual,
                fetch: artifact_fetch,
            },
        );
        let outer = OuterBuilder {
            me: me.clone(),
            // A follower runs no beacon plane, so it starts no agreement instance
            // and has none to adopt.
            agreement_intake: None,
            // Bug A: no-op the blocker on the follower too. The follower spawns no
            // simplex batcher, but it DOES run the marshal cert resolver, whose
            // `deliver=false` verdict would `block!` a registry-∪-committee peer on
            // the shared global transport — the same self-partition the beacon
            // resolver already no-op'd. `provider:` keeps the real oracle.
            blocker: NoopBlocker,
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block: activation,
            signer_keypair: None,
            // A follower runs no agreement plane, so it never MINTS an artifact,
            // and the plane's pull seam does not reach it: that delivers over
            // `BEACON_RESOLVER_CHANNEL` and this node's p2p identity is ephemeral,
            // bootstrapper-less and never tracked into any committee's peer set.
            // What it does have is one delivery route over its cert upstream, so
            // its ladder is no longer permanently empty: vote-only admission now
            // lasts until the epoch's artifact arrives and verifies against
            // `committee[epoch]`, not until the process exits. The multisig quorum
            // is verified either way; the seed check is what it does without in
            // the meantime.
            randomness: randomness.clone(),
            spawn_unblocked: Arc::new(tokio::sync::Notify::new()),
            re_jump,
            soft_enter_committees,
            epoch_metrics: epoch_metrics.clone(),
            executor_metrics: executor_metrics.clone(),
            sync_metrics: sync_metrics.clone(),
            safety_halt: safety_halt.clone(),
            // The follower has no beacon plane, so no tombstone watcher fills a
            // set here. It casts no vote and proposes no block, so both readers of
            // this handle are unreachable on the follower path — an empty set is
            // the honest state, not a lost signal.
            tombstones: crate::slasher::TombstoneSet::default(),
            // A follower runs no beacon plane, hence no finalized poller and no
            // DKG clock — there is no second clock here to diverge from the
            // ordering tip. An unregistered handle publishes nothing, which is
            // the honest answer rather than a lag gauge reading the ordering tip
            // against a permanent zero.
            plane_clock: crate::sync_metrics::PlaneClock::default(),
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
            deque_size: 64,
            partition_prefix: MARSHAL_PARTITION_PREFIX.into(),
            resolver_initial: Duration::from_secs(1),
            resolver_timeout: Duration::from_secs(2),
            resolver_fetch_retry: Duration::from_millis(100),

            genesis: genesis_block,
            beacon_engine: beacon_engine_handle,
            deriver,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            boundary_hook,

            last_execution_finalized_height,
            initial_finalized: (Height::new(anchor_height), anchor_hash),
            initial_head: (Height::new(head_info.best_number), head_info.best_hash),
            marshal_floor: Some(
                jumped_marshal_floor.unwrap_or_else(|| Height::new(finalized_floor)),
            ),
            boundary_fetch,
            // Height-keyed epoch entry, and the read floor a re-jump publishes:
            // both exist to serve `EpochTransition`, which a follower does not
            // run. Its epoch entry rides `boundary_hook` above instead, and it
            // clamps no committee read to a jump landing because the trigger reads
            // at the CURRENT finalized hash, never at a historical one.
            boundary_enter: Arc::new(|_| {}),
            boundary_read_floor: Arc::new(|_| Box::pin(async {})),
            fcu_heartbeat_interval,
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_config.staking_address,
            slasher_reader,
            slasher_latest_finalized_hash,
            slasher_sink: Arc::new(NoopSlasherSink),
            slasher_wal_partition: "slasher-wal".into(),
            // A follower runs no slasher (built, never started) and registers
            // no evidence channel, so there is nothing to bridge to.
            slasher_evidence: None,

            feed,

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine: None,
        }
        // Sibling of the engine context, not a child — see the validator path
        // and `OuterBuilder::build` for why the writer must sit outside the
        // engine's supervision subtree.
        .build(ctx.with_label("outer_engine"))
        .await?;

        // Register the initial epoch's verify-only scheme so the marshal can
        // verify the inlet's certs from cold-start (before any boundary fires).
        let committees_src = crate::cert_inlet::RethCommitteeSource::new(
            RethStakingStateReader::new(
                provider.clone(),
                evm_config.clone(),
                staking_config.clone(),
            ),
            chain_id,
            {
                let p = provider.clone();
                Arc::new(move || {
                    let n = p.finalized_block_number().ok()??;
                    p.block_hash(n).ok().flatten()
                })
            },
        );
        // Follower cold-start register: marshal empty at launch → `cert_seed_pin =
        // None` (vote-only; the accepted residual, upgraded via the inlet cursor).
        if let Ok(scheme) = crate::cert_inlet::CommitteeSource::scheme_at(
            &committees_src,
            initial_epoch_u64,
            anchor_hash,
            None,
        ) {
            outer.cold_start_register(Epoch::new(initial_epoch_u64), scheme);
        }

        // The follower's boundary trigger, consumer half — the twin of the
        // validator path's `epoch_bridge` forwarder, and SUPERVISED for the same
        // reason: if it dies the node keeps following certificates while silently
        // never entering another epoch, so its committee schemes and its repair
        // sweep freeze at whatever the cold-start left.
        //
        // Committee reads run at the CURRENT EL-finalized hash — the same anchor
        // `soft_enter_committees` uses, and the same reason: the committee array
        // and the one-shot keys are frozen storage, so any in-epoch executed hash
        // at or past the commit yields the identical snapshot. `None` (no
        // finalized block yet, or `committee[E]` not committed at that hash) leaves
        // the epoch unconsumed and the next finalized block retries.
        let follower_boundary_tx = outer.boundary_sender();
        let follower_boundary_handle = {
            let canonical = canonical_state.clone();
            let reader = RethStakingStateReader::new(
                provider.clone(),
                evm_config.clone(),
                staking_config.clone(),
            );
            let committee_at: FollowerCommitteeAt = Arc::new(move |epoch: u64| {
                let at = canonical.get_finalized_num_hash()?.hash;
                match reader.epoch_committee_snapshot(epoch, at) {
                    Ok(snap) if !snap.validators.is_empty() => Some(snap),
                    _ => None,
                }
            });
            let deliver: FollowerBoundaryDeliver = Arc::new(move |epoch, snap| {
                let tx = follower_boundary_tx.clone();
                Box::pin(async move { tx.send((epoch, snap)).await.is_ok() })
                    as futures::future::BoxFuture<'static, bool>
            });
            let wake = follower_finalized_wake.clone();
            let height = follower_finalized.clone();
            ctx.with_label("follower_boundary")
                .spawn(move |_| async move {
                    let mut last_delivered: Option<u64> = None;
                    loop {
                        // `notify_one` stores a permit when nobody is waiting, so
                        // a block reported while this task is inside the read
                        // below wakes the NEXT iteration instead of being lost —
                        // the same object-scoped-permit argument the epoch
                        // manager's own edges rest on.
                        let woken = wake.notified();
                        let finalized = height.load(std::sync::atomic::Ordering::Relaxed);
                        if finalized != 0
                            && !enter_finalized_epoch(
                                &mut last_delivered,
                                finalized,
                                activation,
                                interval,
                                &committee_at,
                                &deliver,
                            )
                            .await
                        {
                            error!(
                                "epoch_manager boundary receiver dropped — follower epoch \
                                 entry stopping"
                            );
                            return;
                        }
                        woken.await;
                    }
                })
        };

        // Two clones: one drives the inlet, one is returned to the node for the
        // `consensus`-RPC feed (`set_marshal`/`set_window`).
        let cert_mailbox = outer.marshal_mailbox();
        let inlet_marshal = outer.marshal_mailbox();

        // Start the follower OuterEngine over the ONE broadcast Muxer. The marshal
        // resolver is UPSTREAM-backed (not p2p): it backfills the by-height gap
        // between the cold-start floor and the upstream's live frontier — the gap
        // the inlet's live stream never carries — by pulling each missing height
        // from the cert upstream and delivering it for the marshal to BLS-verify.
        // `upstream` is `Clone` (CertUpstream), so the resolver gets its own handle
        // while the inlet keeps one alive for the live stream + the WS actor. The
        // executor is the sole reth writer from here.
        let consensus_handle = outer.start_follower(broadcast_mux, ctx.clone(), upstream.clone());

        // The cert-inlet — the SOLE producer for a follower. Drives the marshal
        // (which drives the executor) + the B3 serving window. Runs on a child
        // task; fail-closed on TOTAL upstream loss (finalized_rx close).
        // Frontier-aware committee read (the boundary-wedge fix): resolve
        // committee[E] at max(EL-finalized, live-frontier) instead of the lagging
        // finalized tip alone. `live_frontier` is advanced off each BLS-VERIFIED
        // upstream cert via the tee wired below; committee[E] is ahead-committed
        // during epoch E-1 and content-invariant across any in-epoch hash, so
        // reading at the cert-finalized (no-reorg) frontier surfaces it the moment
        // the boundary cert verifies — breaking the producer↔consumer cycle that
        // wedged a migration follower at an epoch boundary (the first cert of epoch
        // E is the very cert that must advance the finalized tip the read was gated
        // on). Mirrors the validator `committee_for` closure in node/dpos.rs.
        let live_frontier = Arc::new(std::sync::atomic::AtomicU64::new(0));
        // Observability for the committee-read defer regimes (the reth
        // pipeline-backfill fail-open): registered ONCE on the launch context so
        // the family carries the launch prefix, then cloned into the closure below
        // (probe_inconsistency) and moved into the inlet (state_not_materialized /
        // committee_not_committed).
        let committee_read_deferred: Family<crate::cert_inlet::CommitteeReadDeferLabels, Counter> =
            Family::default();
        ctx.register(
            "dpos_cert_inlet_committee_read_deferred",
            "Upstream certs deferred (not fatal) on a committee read that could not resolve, \
             by `reason` (state_not_materialized / committee_not_committed / probe_inconsistency).",
            committee_read_deferred.clone(),
        );
        // Stale-cursor observability: verify failures under a CARRY-FORWARD seed
        // pin (a key carried forward from an earlier mint) vs genuine forged-
        // upstream data faults. Registered once here, moved into the inlet.
        let carry_forward_verify_failed = Counter::default();
        ctx.register(
            "dpos_cert_inlet_carry_forward_pin_verify_failed",
            "Upstream cert BLS-verify failures where the seed pin was DERIVED by the \
             ladder (the shared store, or the minting epoch's agreement artifact) \
             rather than already held by the cached scheme.",
            carry_forward_verify_failed.clone(),
        );
        let inlet_committees = crate::cert_inlet::RethCommitteeSource::new(
            RethStakingStateReader::new(
                provider.clone(),
                evm_config.clone(),
                staking_config.clone(),
            ),
            chain_id,
            {
                let p = provider.clone();
                let live_frontier = live_frontier.clone();
                let deferred = committee_read_deferred.clone();
                // Once-per-episode rate limit for the probe-fault `error!` below;
                // cleared on the next fault-free probe round.
                let probe_fault_logged = std::sync::atomic::AtomicBool::new(false);
                Arc::new(move || {
                    let fin = p.finalized_block_number().ok().flatten();
                    let live = live_frontier.load(std::sync::atomic::Ordering::Relaxed);
                    // No finalized marker AND no live cursor yet ⇒ not readable
                    // (a `unwrap_or(0)` would misread committee at genesis).
                    if fin.is_none() && live == 0 {
                        return None;
                    }
                    let fin = fin.unwrap_or(0);
                    let read_at = fin.max(live);
                    // Gate the committee-read anchor on reth's MATERIALIZED head via
                    // the shared `executed_state_hash` probe (executed.rs): a
                    // `block_hash` alone is a HEADER probe that resolves `Some` for a
                    // height whose state is NOT yet materialized (pipeline backfill
                    // writes headers ahead of state), so feeding it drives the
                    // committee read at an un-executed hash → StateForHashNotFound.
                    // `read_at` above `best_block_number()` → `Ok(None)` here, and we
                    // fall back to the finalized hash (`fin` is materialized once best
                    // catches it; while best still trails fin — deep backfill — that
                    // read is `Ok(None)` too and the inlet defers SILENTLY). A probe
                    // `Err` is DIFFERENT: a `block_hash` miss at a MATERIALIZED height
                    // (executed.rs deliberately surfaces it as header-index
                    // inconsistency/corruption). The follower never-crash posture still
                    // DEFERS (the fallback may resolve; a `None` return can never
                    // select a WRONG committee), but LOUDLY: `error!` once per episode
                    // + a distinct `reason="probe_inconsistency"` counter tick — NOT
                    // folded into the silent above-best park. NB: best_block_number,
                    // NOT last_block_number, is correct for state-read anchoring (it
                    // trails during backfill — the deliberate defer signal).
                    let (resolved, probe_err) = match executed_state_hash(&p, read_at) {
                        Ok(Some(h)) => (Some(h), None),
                        Ok(None) => match executed_state_hash(&p, fin) {
                            Ok(h) => (h, None),
                            Err(e) => (None, Some(e)),
                        },
                        Err(e) => (executed_state_hash(&p, fin).ok().flatten(), Some(e)),
                    };
                    match probe_err {
                        Some(e) => {
                            deferred
                                .get_or_create(&crate::cert_inlet::CommitteeReadDeferLabels {
                                    reason: crate::cert_inlet::DEFER_PROBE_INCONSISTENCY,
                                })
                                .inc();
                            if !probe_fault_logged.swap(true, std::sync::atomic::Ordering::Relaxed)
                            {
                                error!(
                                    error = %e,
                                    read_at,
                                    fin,
                                    "cert-inlet committee-read anchor probe failed at a \
                                     materialized height (header-index inconsistency); deferring \
                                     committee reads DEGRADED-LOUD, not fatal (rate-limited; \
                                     dpos_cert_inlet_committee_read_deferred_total ticks per read)"
                                );
                            }
                        }
                        None => {
                            probe_fault_logged.store(false, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    resolved
                })
            },
        );
        let shutdown_for_inlet = shutdown.clone();
        // The SAME provider the epoch manager holds, not a second one over a
        // private store — and on this path that is now load-bearing twice over.
        // `observe_cert` prunes what `pin_for` reads, so splitting them would make
        // the pruning a no-op on a map nothing else can see; and `observe_cert` is
        // also the key-delivery TRIGGER, so a second instance would fetch into a
        // store the epoch manager's repair sweep never reads.
        let inlet_randomness = randomness.clone();
        // DATA-fault rotation trigger (#7): after MAX_UPSTREAM_FAULTS consecutive
        // unverifiable certs over a healthy connection the inlet rotates to the
        // next configured upstream URL (connection-level failover can never see a
        // bad PAYLOAD on a live connection). Built from a clone of the SAME
        // `CertUpstream` handle the keepalive holds; `rotate()` drops the
        // connection so the WS actor's run loop advances to the next URL.
        let inlet_rotate: Option<crate::cert_inlet::RotateUpstream> = upstream
            .as_ref()
            .map(crate::cert_follow::CertUpstream::rotate_callback);
        // SUPERVISED, not detached — the follower twin of the validator path's
        // `("inlet", h)` registration (`node/src/cert_inlet.rs` → `dpos.rs`).
        // The `shutdown.cancel()` at the end of the body covers the LOOP exits
        // (total upstream loss / committee fatal); a PANIC skips it entirely and
        // `with_catch_panics(true)` swallows it, which on this path means zero
        // certificate ingestion with a live network and a live RPC — the node
        // looks healthy and follows nothing. The returned handle resolves
        // `Err(Error::Exited)` on that panic, so the host supervisor fails the
        // node closed exactly as the loop exits already do.
        let cert_inlet_handle = ctx.with_label("cert_inlet").spawn(move |c| async move {
            // Hold the WS upstream REQUEST handle alive for the inlet's whole
            // lifetime. The WS actor's `run` loop exits the instant ALL
            // `UpstreamHandle`s drop (its `mailbox_rx` closes → `None => return`),
            // and that SAME actor feeds `finalized_rx` (the inlet's sole producer).
            // The cold-start only borrows it (`get_latest`/jump), so without this
            // move it would drop when `launch_follower` returns → the WS actor
            // exits cleanly → the node-stack supervisor tears the follower down
            // before it ever follows. The marshal's `UpstreamResolver` holds its
            // OWN clone of this same handle for by-height gap-repair pulls; this one
            // keeps the live stream + WS actor alive.
            let _upstream_keepalive = upstream;
            // B4: `--cert-follow` is ALWAYS verify (the inlet has no no-verify mode).
            // `with_epoch_math` arms the defense-in-depth height↔epoch bind: the
            // follower fully trusts upstream committee reads, so it binds each
            // cert's round-epoch to its block's height-derived epoch.
            //
            // The live-frontier tee is wired ONLY for its `live_height` cursor (the
            // frontier-aware committee read above — the boundary-wedge fix). The
            // follower has no beacon plane, so the DkgActor deal clock is a no-op:
            // drop the receiver and each `try_send` is a benign Closed.
            let (dkg_tx, dkg_rx) = tokio::sync::mpsc::channel::<u64>(1);
            drop(dkg_rx);
            let mut inlet = crate::cert_inlet::CertInlet::new(inlet_marshal, inlet_committees, c)
                .with_epoch_math(activation, interval)
                .with_committee_read_deferred_metric(committee_read_deferred)
                .with_carry_forward_fail_metric(carry_forward_verify_failed)
                .with_randomness(inlet_randomness)
                .with_tee(crate::cert_inlet::LiveFrontierTee {
                    live_height: live_frontier,
                    // Same atomic the steady-state re-jump trigger reads: the inlet
                    // advances it on every cert (deferred ones included) so a frozen
                    // marshal tip can't freeze the re-jump (the cascade-wedge fix).
                    upstream_frontier: upstream_frontier.clone(),
                    dkg_height_tx: dkg_tx,
                });
            if let Some(rotate) = inlet_rotate {
                inlet = inlet.with_rotate(rotate);
            }
            // Per-connection data-fault scoping (#7): a connection-level
            // auto-rotation (the WS actor reconnecting to the next URL on a
            // dropped/failed connection) bumps `conn_gen`; the inlet resets its
            // streak on the change so A's faults never bleed into B's budget.
            if let Some(conn_gen) = conn_gen {
                inlet = inlet.with_connection_token(conn_gen);
            }
            if let Some(tx) = verified_tx {
                inlet = inlet.with_window(tx);
            }
            let mut finalized_rx = finalized_rx;
            info!("cert-inlet follower producer started");
            loop {
                match finalized_rx.recv().await {
                    Some(uf) => {
                        if let Err(e) = inlet.ingest(uf).await {
                            error!(error = ?e, "cert-inlet fatal (committee read); fail-closed");
                            break;
                        }
                    }
                    None => {
                        error!(
                            "cert-inlet WS stream closed (all upstreams dead); exiting fail-closed"
                        );
                        break;
                    }
                }
            }
            // TOTAL upstream loss is fail-closed (Risk-3): cancel the shared
            // shutdown so the host brings the node down (case-cert-cascade A3
            // accepts an `exited` state; a silent hang would fail it).
            shutdown_for_inlet.cancel();
        });

        Ok(DposLayerHandle {
            consensus_handle,
            cert_mailbox,
            supervised: vec![
                ("cert_inlet", cert_inlet_handle),
                ("follower_boundary", follower_boundary_handle),
                // SUPERVISED, not detached: it parks rather than returning, so a
                // clean exit means it died — and a dead fetcher silently returns
                // this node to vote-only admission for the rest of the process,
                // with every liveness check still green.
                ("follower_artifact_fetch", artifact_fetch_handle),
            ],
            // A follower owns no journal: both were deleted with its key store
            // (see where `randomness` is built above), so there is nothing to
            // drain here.
            drain_on_shutdown: vec![],
            artifact_bytes: Some(artifact_bytes),
        })
    }
}

#[cfg(test)]
mod cold_start_kind_tests {
    use super::{
        classify_jump_outcome, cold_start_jump_eligible, resolve_cold_start_kind, ColdStartKind,
        JumpDisposition,
    };
    use crate::cold_start_jump::JumpOutcome;
    use alloy_primitives::B256;

    const ACTIVATION: u64 = 192;
    const INTERVAL: u32 = 64;

    // The cold-start disposition split (#11): `Stalled` now routes to a
    // retry-forever no-peers self-heal (was a FATAL re-fuse); every other outcome
    // keeps its terminal mapping. The steady-state classifier is tested in
    // cold_start_jump.rs.
    #[test]
    fn classify_jump_outcome_dispositions() {
        let hash = B256::repeat_byte(0x5A);
        let done = |o| match classify_jump_outcome(o) {
            JumpDisposition::Done(res) => res,
            JumpDisposition::RetryStalled(_) => panic!("expected Done, got RetryStalled"),
            JumpDisposition::RotateAuth(_) => panic!("expected Done, got RotateAuth"),
        };
        assert_eq!(
            done(JumpOutcome::Landed {
                landing: 700,
                hash,
                floor: 697
            })
            .expect("Landed is Ok"),
            Some((700, hash, 697)),
            "Landed ⇒ Done(Ok(Some(landing, hash, floor)))"
        );
        assert_eq!(
            done(JumpOutcome::Lagging).expect("Lagging is Ok"),
            None,
            "Lagging ⇒ Done(Ok(None)) (no-op; inlet pulls cover the residual gap)"
        );
        assert_eq!(
            done(JumpOutcome::BadTarget(eyre::eyre!("payload != digest")))
                .expect("BadTarget is Ok"),
            None,
            "BadTarget ⇒ Done(Ok(None)) (forgeable pre-anchor mismatch — boot anyway)"
        );
        assert_eq!(
            done(JumpOutcome::InvalidTarget(eyre::eyre!("reth REJECTED"))).expect("Invalid is Ok"),
            None,
            "InvalidTarget ⇒ Done(Ok(None)) (reth-rejected branch mid-sync — boot anyway)"
        );
        // A forged/unagreed POST-sync branch now ROTATES
        // the upstream + re-jumps (was fail-closed `Done(Err)`) — never crash on a
        // forged UPSTREAM.
        assert!(
            matches!(
                classify_jump_outcome(JumpOutcome::AuthFailed(eyre::eyre!("forged branch"))),
                JumpDisposition::RotateAuth(_)
            ),
            "AuthFailed ⇒ RotateAuth (rotate + backoff + re-jump, not fail-closed)"
        );
        // #11: a transport stall (likely zero peers) is now a retry, NOT a fatal.
        assert!(
            matches!(
                classify_jump_outcome(JumpOutcome::Stalled(eyre::eyre!("transport stall"))),
                JumpDisposition::RetryStalled(_)
            ),
            "Stalled ⇒ RetryStalled (no-peers self-heal — retry forever, not fatal)"
        );
        // Connected-but-wedged EL pipeline (soak v43): at cold-start, retry forever
        // (same posture as `Stalled`) — a deterministic re-wedge stays observable +
        // un-advanced rather than crashing the node.
        assert!(
            matches!(
                classify_jump_outcome(JumpOutcome::StalledWithPeers(eyre::eyre!("EL wedged"))),
                JumpDisposition::RetryStalled(_)
            ),
            "StalledWithPeers ⇒ RetryStalled (connected-but-wedged self-heal — retry, not fatal)"
        );
    }

    #[test]
    fn fresh_migration_never_jumps_even_with_an_upstream() {
        // The load-bearing #7 guard: a FreshMigration MUST anchor at
        // dposActivationBlock (clean-halt invariant), so the cold-start jump is
        // inert for it regardless of whether an upstream is configured.
        assert!(!cold_start_jump_eligible(
            ColdStartKind::FreshMigration,
            true
        ));
        assert!(!cold_start_jump_eligible(
            ColdStartKind::FreshMigration,
            false
        ));
    }

    #[test]
    fn restart_jumps_only_with_an_upstream() {
        // A no-upstream validator catches up on the consensus-plane treadmill,
        // NOT via the jump (Risk-1).
        assert!(cold_start_jump_eligible(ColdStartKind::Restart, true));
        assert!(!cold_start_jump_eligible(ColdStartKind::Restart, false));
    }

    #[test]
    fn overshoot_with_empty_archive_and_no_upstream_is_fatal() {
        let err =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 500, false).unwrap_err();
        assert!(
            err.to_string().contains("--dpos.follower-upstream"),
            "{err}"
        );
    }

    #[test]
    fn overshoot_with_empty_archive_and_upstream_is_legal_deep_restart() {
        // With a sync upstream (plane-tracked or WS) the EL-past-epoch-0 empty-archive
        // arm is a legal deep `Restart`: the pre-engine `cold_start_jump` re-seeds the
        // anchor. The launch caller then REQUIRES that jump to land.
        let kind = resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 500, true)
            .expect("legal deep restart with an upstream");
        assert_eq!(kind, ColdStartKind::Restart);
    }

    #[test]
    fn inside_epoch_zero_is_fresh_migration() {
        let kind =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 10, true).expect("fresh");
        assert_eq!(kind, ColdStartKind::FreshMigration);
    }

    #[test]
    fn archive_present_is_restart() {
        // A populated archive resumes at its finalized height regardless of upstream.
        let kind = resolve_cold_start_kind(
            ACTIVATION + 300,
            ACTIVATION,
            INTERVAL,
            ACTIVATION + 500,
            false,
        )
        .expect("restart");
        assert_eq!(kind, ColdStartKind::Restart);
    }

    #[test]
    fn boundary_exactly_one_interval_is_overshoot() {
        // cs_finalized == activation + interval is the FIRST fatal height
        // (epoch 0 is [activation, activation + interval)) when there is no upstream.
        let err =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + INTERVAL as u64, false)
                .unwrap_err();
        assert!(err.to_string().contains("past epoch 0"), "{err}");
    }

    #[test]
    fn zero_activation_is_the_unscheduled_sentinel_and_fatal() {
        let err = resolve_cold_start_kind(0, 0, INTERVAL, 0, true).unwrap_err();
        assert!(err.to_string().contains("unscheduled sentinel"), "{err}");
    }
}

#[cfg(test)]
mod broker_repromote_tests {
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_p2p::{
        simulated::{Config as SimConfig, Link, Network},
        utils::mux::Muxer,
        Manager as _, Receiver as _, Recipients, Sender as _,
    };
    use commonware_runtime::{deterministic, IoBuf, Metrics as _, Quota, Runner as _};
    use commonware_utils::{ordered::Set, NZUsize};
    use std::{num::NonZeroU32, sync::Arc, time::Duration};
    use tokio::sync::Mutex;

    const SUBCHANNEL: u64 = 0;
    const QUOTA: Quota = Quota::per_second(NonZeroU32::MAX);

    /// A demoted engine drops its `SubReceiver`s; a re-promoted engine CLONES the
    /// plane's `Arc<Mutex<MuxHandle>>` (the `PlaneMux` sharing) and re-registers the
    /// SAME subchannel against the SAME persistent broker — no network rebuild. This
    /// is the restart-free re-promotion property the broker-in-plane refactor adds.
    #[test]
    fn plane_mux_supports_drop_then_reclone_reregister() {
        let executor = deterministic::Runner::default();
        executor.start(|ctx| async move {
            let (network, oracle) = Network::<_, super::PeerPubkey>::new(
                ctx.with_label("net"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(1),
                },
            );
            network.start();

            let (pk1, pk2) = (
                PrivateKey::from_seed(1).public_key(),
                PrivateKey::from_seed(2).public_key(),
            );
            oracle
                .manager()
                .track(0, Set::from_iter_dedup([pk1.clone(), pk2.clone()]))
                .await;
            for (a, b) in [(pk1.clone(), pk2.clone()), (pk2.clone(), pk1.clone())] {
                oracle
                    .add_link(
                        a,
                        b,
                        Link {
                            latency: Duration::from_millis(0),
                            jitter: Duration::from_millis(0),
                            success_rate: 1.0,
                        },
                    )
                    .await
                    .unwrap();
            }

            // Plane-side broker over peer 1; the plane shares the registrar behind
            // Arc<Mutex> (PlaneMux).
            let (s1, r1) = oracle
                .control(pk1.clone())
                .register(7, QUOTA)
                .await
                .unwrap();
            let (mux1, handle1) = Muxer::new(ctx.with_label("mux1"), s1, r1, 8);
            mux1.start();
            let plane_mux = Arc::new(Mutex::new(handle1));

            // Sender side over peer 2.
            let (s2, r2) = oracle
                .control(pk2.clone())
                .register(7, QUOTA)
                .await
                .unwrap();
            let (mux2, mut handle2) = Muxer::new(ctx.with_label("mux2"), s2, r2, 8);
            mux2.start();
            let (mut tx2, _rx2) = handle2.register(SUBCHANNEL).await.unwrap();

            // Two promotions: each clones the SAME plane_mux, registers SUBCHANNEL,
            // receives, then drops its SubReceiver (auto-deregister) at scope exit.
            for payload in [b"a".as_ref(), b"b".as_ref()] {
                let p = plane_mux.clone();
                let (_sub_tx, mut sub_rx) = p.lock().await.register(SUBCHANNEL).await.unwrap();
                tx2.send(Recipients::One(pk1.clone()), IoBuf::from(payload), false)
                    .await
                    .unwrap();
                let (from, _) = sub_rx.recv().await.unwrap();
                assert_eq!(from, pk2);
            }
        });
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
            proposal_view: 0,
            timestamp: 7,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            parent_seed: None,
            equivocation: None,
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
            _seed: Option<crate::beacon::seed::Seed>,
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

#[cfg(test)]
mod self_heal_tests {
    use super::{read_with_visibility_belt, wait_for_activation_block, SyncMetrics, SyncReason};
    use alloy_primitives::{BlockNumber, B256};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_storage_api::{errors::provider::ProviderResult, BlockHashReader};
    use std::sync::atomic::{AtomicU32, Ordering};

    // #17 visibility belt: a landing read that returns `None` a few times then
    // materializes must resolve (not fatal), raise `landing_wait` while waiting,
    // and clear it on success.
    #[test]
    fn belt_retries_then_resolves_and_clears_the_gauge() {
        deterministic::Runner::default().start(|ctx| async move {
            let m = SyncMetrics::default();
            let calls = AtomicU32::new(0);
            let hash = B256::repeat_byte(0x11);
            let got = read_with_visibility_belt(&ctx, &m, "the landing", || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n >= 1 {
                    // The first miss must already have raised the gauge.
                    assert_eq!(m.degraded_value(SyncReason::LandingWait), 1);
                }
                Ok(if n < 2 { None } else { Some(hash) })
            })
            .await
            .expect("belt resolves once the block materializes");
            assert_eq!(got, hash);
            assert!(
                calls.load(Ordering::SeqCst) >= 3,
                "retried before resolving"
            );
            assert_eq!(
                m.degraded_value(SyncReason::LandingWait),
                0,
                "cleared on success"
            );
        });
    }

    // #17 belt: a genuinely materialized-but-missing read stays fatal AFTER the
    // belt expires (not retry-forever — a landing local fault, not correlated).
    #[test]
    fn belt_expiry_is_fatal_and_clears_the_gauge() {
        deterministic::Runner::default().start(|ctx| async move {
            let m = SyncMetrics::default();
            let err = read_with_visibility_belt(&ctx, &m, "the landing", || {
                Ok::<_, eyre::Report>(None::<B256>)
            })
            .await
            .expect_err("a never-materializing read is fatal past the belt");
            assert!(err.to_string().contains("visibility belt"), "{err}");
            assert_eq!(
                m.degraded_value(SyncReason::LandingWait),
                0,
                "cleared on expiry"
            );
        });
    }

    /// Returns `None` for the first `misses` reads, then `present`.
    struct FlakyHashReader {
        calls: AtomicU32,
        misses: u32,
        present: B256,
        metrics: SyncMetrics,
    }

    impl BlockHashReader for FlakyHashReader {
        fn block_hash(&self, _number: BlockNumber) -> ProviderResult<Option<B256>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n >= 1 && n < self.misses {
                // Every wait iteration keeps `activation_wait` raised.
                assert_eq!(self.metrics.degraded_value(SyncReason::ActivationWait), 1);
            }
            Ok(if n < self.misses {
                None
            } else {
                Some(self.present)
            })
        }
        fn canonical_hashes_range(
            &self,
            _start: BlockNumber,
            _end: BlockNumber,
        ) -> ProviderResult<Vec<B256>> {
            Ok(Vec::new())
        }
    }

    // #13 activation-wait: a transient absence RETRIES (the old code fatal-ed after
    // a 120 s deadline; the deadline is now gone — retry-forever) and resolves when
    // the sequencer's block lands, raising `activation_wait` while it waits and
    // clearing it on success. Never fatal on a mere transient absence.
    #[test]
    fn activation_wait_retries_then_resolves() {
        deterministic::Runner::default().start(|ctx| async move {
            let m = SyncMetrics::default();
            let present = B256::repeat_byte(0xA7);
            let provider = FlakyHashReader {
                calls: AtomicU32::new(0),
                misses: 4,
                present,
                metrics: m.clone(),
            };
            let got = wait_for_activation_block(&ctx, &provider, 192, &m)
                .await
                .expect("activation wait resolves, never fatal");
            assert_eq!(got, present);
            assert!(
                provider.calls.load(Ordering::SeqCst) >= 5,
                "retried on each transient miss instead of failing fast"
            );
            assert_eq!(
                m.degraded_value(SyncReason::ActivationWait),
                0,
                "cleared on success"
            );
        });
    }
}

#[cfg(test)]
mod crash_recover_tests {
    use super::{
        crash_recover_defer_or_fatal, recover_reconnect_point, ReconnectScan, RecoverOutcome,
        SyncMetrics, SyncReason,
    };
    use alloy_primitives::{BlockNumber, B256};
    use reth_chainspec::ChainInfo;
    use reth_storage_api::{errors::provider::ProviderResult, BlockHashReader, BlockNumReader};
    use std::collections::HashSet;

    /// reth holds exactly the heights in `present`; `best` is its canonical tip.
    struct SparseProvider {
        present: HashSet<u64>,
        best: u64,
    }

    impl BlockHashReader for SparseProvider {
        fn block_hash(&self, number: BlockNumber) -> ProviderResult<Option<B256>> {
            Ok(self
                .present
                .contains(&number)
                .then(|| B256::repeat_byte(number as u8)))
        }
        fn canonical_hashes_range(
            &self,
            _start: BlockNumber,
            _end: BlockNumber,
        ) -> ProviderResult<Vec<B256>> {
            Ok(Vec::new())
        }
    }

    impl BlockNumReader for SparseProvider {
        fn chain_info(&self) -> ProviderResult<ChainInfo> {
            Ok(ChainInfo::default())
        }
        fn best_block_number(&self) -> ProviderResult<BlockNumber> {
            Ok(self.best)
        }
        fn last_block_number(&self) -> ProviderResult<BlockNumber> {
            Ok(self.best)
        }
        fn block_number(&self, _hash: B256) -> ProviderResult<Option<BlockNumber>> {
            Ok(None)
        }
    }

    // #12: reth missing a small tail below `target` reconnects at the highest
    // present ancestor and replays it (the flush-race path — NOT a defer).
    #[test]
    fn small_gap_reconnects_for_replay() {
        // reth holds up to 998; target 1000 (999, 1000 missing) → reconnect at 999.
        let provider = SparseProvider {
            present: (0..=998).collect(),
            best: 998,
        };
        match recover_reconnect_point(&provider, 1000, 64).expect("scan ok") {
            ReconnectScan::Reconnect(lowest) => assert_eq!(lowest, 999),
            ReconnectScan::TooDeep => panic!("a 2-block tail must NOT be TooDeep"),
        }
    }

    // #12: reth missing MORE than MAX_COLD_RECOVER below its own archive is TooDeep —
    // the pre-engine replay can't bridge it; the recover fn defers to devp2p EL-sync.
    #[test]
    fn deep_gap_is_too_deep_for_replay() {
        // reth holds only up to 100; target 1000 → far more than 64 behind.
        let provider = SparseProvider {
            present: (0..=100).collect(),
            best: 100,
        };
        assert!(
            matches!(
                recover_reconnect_point(&provider, 1000, 64).expect("scan ok"),
                ReconnectScan::TooDeep
            ),
            "a > 64-block gap is TooDeep (#12 defer, not a flush-race replay)"
        );
    }

    // #12 disposition WITH an upstream: DEFER to EL-sync + raise the crash-recover
    // gauges/counter (the caller then anchors at reth's tip so the jump devp2p-backfills
    // the EL — the archive is INTACT, so no consensus-store repair is needed).
    #[test]
    fn defer_with_upstream_raises_gauges_and_reports_gap() {
        let provider = SparseProvider {
            present: (0..=100).collect(),
            best: 100,
        };
        let m = SyncMetrics::default();
        let outcome =
            crash_recover_defer_or_fatal(&provider, 1000, true, &m, "test gap").expect("defers");
        match outcome {
            RecoverOutcome::DeferToElSync { gap } => assert_eq!(gap, 900, "target 1000 − best 100"),
            RecoverOutcome::Recovered(_) => panic!("must defer, not recover"),
        }
        assert_eq!(
            m.degraded_value(SyncReason::CrashRecover),
            1,
            "crash_recover gauge raised"
        );
        assert_eq!(
            m.crash_recover_deferred_to_elsync.get(),
            1,
            "defer counter incremented"
        );
        assert_eq!(m.crash_recover_gap_blocks.get(), 900, "gap gauge set");
    }

    // #12 residual fatal WITHOUT an upstream: there is nowhere to devp2p-backfill from —
    // real local data loss stays FATAL (Decision A does not apply to idiosyncratic local
    // corruption), and no gauge is raised.
    #[test]
    fn defer_without_upstream_is_fatal() {
        let provider = SparseProvider {
            present: (0..=100).collect(),
            best: 100,
        };
        let m = SyncMetrics::default();
        let err = crash_recover_defer_or_fatal(&provider, 1000, false, &m, "test gap")
            .expect_err("no upstream ⇒ fatal");
        assert!(
            err.to_string().contains("--dpos.follower-upstream"),
            "{err}"
        );
        assert_eq!(
            m.degraded_value(SyncReason::CrashRecover),
            0,
            "the residual-fatal path raises no self-heal gauge"
        );
    }
}

// #8: the below-floor archive-hole heal is NOT a defer — with an upstream it
// RE-POPULATES the hole by a BLS-verified by-height re-fetch. These pin
// `refetch_verified_archive_hole` in isolation (the disk-archive replay it splices
// into is exercised by the smoke suite): a valid cert heals, a no-upstream / gone-
// everywhere / forged cert is fatal.
#[cfg(test)]
mod refetch_hole_tests {
    use super::refetch_verified_archive_hole;
    use crate::{
        cert_follow::{CertUpstream, UpstreamFinalized},
        cert_inlet::CommitteeSource,
        digest::Digest,
        order_block::OrderBlock,
    };
    use alloy_primitives::{Address, Bytes, B256};
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Height, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_parallel::Sequential;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::BiMap, TryCollect as _};
    use fluentbase_bls::{
        beacon::GroupPublic, fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
        scheme::build_verifier, BlsPubkey, PeerPubkey, Scheme as BlsScheme,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;

    struct Committee {
        signers: Vec<BlsScheme>,
        verifier: BlsScheme,
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
                use commonware_codec::DecodeExt as _;
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                )
            })
            .try_collect()
            .unwrap();
        let ns = fluent_namespace(CHAIN_ID);
        let signers = bls_kps
            .iter()
            .map(|kp| build_signer(&ns, bimap.clone(), kp, None).expect("member"))
            .collect();
        let verifier = build_verifier(&ns, bimap, None, None);
        Committee { signers, verifier }
    }

    fn sample_order(height: u64) -> OrderBlock {
        OrderBlock {
            parent: Digest(B256::repeat_byte(0xaa)),
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            parent_seed: None,
            equivocation: None,
        }
    }

    /// A real 2f+1 finalization cert over `block`'s digest, signed by `c`.
    fn certify(c: &Committee, epoch: u64, block: &OrderBlock) -> UpstreamFinalized {
        let round = Round::new(Epoch::new(epoch), View::new(block.height));
        let prop = Proposal::new(round, View::new(block.height - 1), block.digest());
        let finalizes: Vec<_> = c
            .signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        let finalization = Finalization::from_finalizes(&c.verifier, finalizes.iter(), &Sequential)
            .expect("quorum");
        UpstreamFinalized {
            finalization,
            block: block.clone(),
        }
    }

    /// Returns a canned `UpstreamFinalized` (or `None`) for any height requested.
    #[derive(Clone)]
    struct FakeUpstream(Option<UpstreamFinalized>);
    impl CertUpstream for FakeUpstream {
        async fn get_finalization(&self, _height: Height) -> Option<UpstreamFinalized> {
            self.0.clone()
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            self.0.clone()
        }
        async fn rotate(&self) {}
    }

    /// Hands out a fixed verifier for every epoch (the trust anchor under test).
    struct CannedCommittees(BlsScheme);
    impl CommitteeSource for CannedCommittees {
        fn scheme_at(
            &self,
            _epoch: u64,
            _at_hash: B256,
            _pin: Option<GroupPublic>,
        ) -> eyre::Result<BlsScheme> {
            Ok(self.0.clone())
        }
        fn scheme_at_finalized_tip(
            &self,
            _epoch: u64,
            _pin: Option<GroupPublic>,
        ) -> eyre::Result<Option<BlsScheme>> {
            Ok(Some(self.0.clone()))
        }
    }

    // WITH an upstream serving a committee-signed cert, a below-floor hole is
    // RE-POPULATED (returns the verified block+cert to splice into the replay),
    // never deferred.
    #[test]
    fn valid_upstream_cert_repopulates_the_hole() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(1);
            let block = sample_order(65);
            let uf = certify(&c, 0, &block);
            let up = FakeUpstream(Some(uf));
            let committees = CannedCommittees(c.verifier.clone());
            let out = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                None,
                65,
                "finalized_blocks",
            )
            .await
            .expect("valid cert heals the hole");
            assert_eq!(
                out.block.height, 65,
                "the re-fetched block is returned for replay"
            );
        });
    }

    // WITHOUT an upstream: a below-floor hole is unrecoverable local data loss → FATAL.
    #[test]
    fn no_upstream_is_fatal() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(2);
            let committees = CannedCommittees(c.verifier);
            let err = refetch_verified_archive_hole::<FakeUpstream, _>(
                None,
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                None,
                65,
                "finalized_blocks",
            )
            .await
            .err()
            .expect("no upstream ⇒ fatal");
            assert!(
                err.to_string().contains("--dpos.follower-upstream"),
                "{err}"
            );
        });
    }

    // Upstream reachable but no longer serves the below-floor height (pruned
    // everywhere) → FATAL (gone-everywhere).
    #[test]
    fn upstream_missing_height_is_fatal() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(3);
            let up = FakeUpstream(None);
            let committees = CannedCommittees(c.verifier);
            let err = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                None,
                65,
                "finalized_blocks",
            )
            .await
            .err()
            .expect("upstream lacks the height ⇒ fatal");
            assert!(err.to_string().contains("gone everywhere"), "{err}");
        });
    }

    // A forged cert (signed by a DIFFERENT committee than the one the trust anchor
    // reads) FAILS the BLS authentication → FATAL: the re-fetch cannot be steered by
    // a malicious upstream.
    #[test]
    fn forged_cert_fails_authentication() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let signer_committee = committee(4);
            let trust_committee = committee(5); // a DIFFERENT committee
            let block = sample_order(65);
            let uf = certify(&signer_committee, 0, &block);
            let up = FakeUpstream(Some(uf));
            let committees = CannedCommittees(trust_committee.verifier);
            let err = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                None,
                65,
                "finalized_blocks",
            )
            .await
            .err()
            .expect("cert signed by the wrong committee ⇒ auth failure");
            // The inner cause is the BLS-verify `ensure!`; assert on the full chain.
            assert!(
                format!("{err:#}").contains("FAILED BLS verification"),
                "{err:#}"
            );
        });
    }
}

#[cfg(test)]
mod follower_boundary_tests {
    use super::{enter_finalized_epoch, FollowerBoundaryDeliver, FollowerCommitteeAt};
    use alloy_primitives::B256;
    use commonware_consensus::types::Epoch;
    use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
    use std::sync::{Arc, Mutex};

    const ACTIVATION: u64 = 100;
    const INTERVAL: u32 = 10;

    fn snapshot(epoch: u64) -> ValidatorSetSnapshot {
        ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x33),
            block_number: 7,
            epoch,
            validators: Vec::new(),
            weights: None,
        }
    }

    /// Records what reached the epoch manager, and lets a test make the committee
    /// read or the delivery fail on demand.
    #[derive(Default)]
    struct Recorder {
        delivered: Mutex<Vec<u64>>,
        committee_readable: Mutex<bool>,
        receiver_alive: Mutex<bool>,
    }

    fn seams(
        readable: bool,
        alive: bool,
    ) -> (Arc<Recorder>, FollowerCommitteeAt, FollowerBoundaryDeliver) {
        let rec = Arc::new(Recorder {
            delivered: Mutex::new(Vec::new()),
            committee_readable: Mutex::new(readable),
            receiver_alive: Mutex::new(alive),
        });
        let for_read = rec.clone();
        let committee_at: FollowerCommitteeAt = Arc::new(move |epoch| {
            (*for_read.committee_readable.lock().unwrap()).then(|| snapshot(epoch))
        });
        let for_deliver = rec.clone();
        let deliver: FollowerBoundaryDeliver = Arc::new(move |epoch: Epoch, _snap| {
            let rec = for_deliver.clone();
            Box::pin(async move {
                if !*rec.receiver_alive.lock().unwrap() {
                    return false;
                }
                rec.delivered.lock().unwrap().push(epoch.get());
                true
            }) as futures::future::BoxFuture<'static, bool>
        });
        (rec, committee_at, deliver)
    }

    /// `boundary_hook` fires per finalized block — ~1/s in production — while
    /// `reconcile_roles` prunes, re-registers and sweeps on each delivery. Reds if
    /// the once-per-epoch gate is dropped.
    #[tokio::test]
    async fn every_finalized_block_of_one_epoch_delivers_one_boundary() {
        let (rec, committee_at, deliver) = seams(true, true);
        let mut last = None;

        for height in [100, 103, 109, 110, 117] {
            assert!(
                enter_finalized_epoch(
                    &mut last,
                    height,
                    ACTIVATION,
                    INTERVAL,
                    &committee_at,
                    &deliver
                )
                .await
            );
        }

        assert_eq!(*rec.delivered.lock().unwrap(), vec![0, 1]);
        assert_eq!(last, Some(1));
    }

    /// `committee[E]` is committed during `E-1`, but the read runs at the
    /// EL-finalized hash, which trails the ordering-finalized height by K — so the
    /// first blocks of `E` can legitimately read back nothing. Consuming the epoch
    /// there would skip its scheme registration for the whole epoch.
    ///
    /// Reds if `last_delivered` advances on an unreadable committee.
    #[tokio::test]
    async fn an_unreadable_committee_leaves_the_epoch_for_the_next_block() {
        let (rec, committee_at, deliver) = seams(false, true);
        let mut last = None;

        assert!(
            enter_finalized_epoch(
                &mut last,
                100,
                ACTIVATION,
                INTERVAL,
                &committee_at,
                &deliver
            )
            .await
        );
        assert!(rec.delivered.lock().unwrap().is_empty());
        assert_eq!(last, None);

        *rec.committee_readable.lock().unwrap() = true;
        assert!(
            enter_finalized_epoch(
                &mut last,
                101,
                ACTIVATION,
                INTERVAL,
                &committee_at,
                &deliver
            )
            .await
        );
        assert_eq!(*rec.delivered.lock().unwrap(), vec![0]);
    }

    /// A closed boundary receiver means the epoch manager has exited. The trigger
    /// stops and the supervisor takes the node down, rather than spinning on a
    /// dead channel for every finalized block.
    ///
    /// Reds if the send result stops being propagated.
    #[tokio::test]
    async fn a_dropped_boundary_receiver_stops_the_trigger() {
        let (_rec, committee_at, deliver) = seams(true, false);
        let mut last = None;

        assert!(
            !enter_finalized_epoch(
                &mut last,
                100,
                ACTIVATION,
                INTERVAL,
                &committee_at,
                &deliver
            )
            .await
        );
        assert_eq!(last, None, "an undelivered epoch must not be consumed");
    }
}
