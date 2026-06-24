//! DPoS layer launcher — assembles 03 (staking-reader), 04 (consensus),
//! and 05 (p2p) given operator keys, reth handles, and config. Spawned
//! by the host adapter at `crates/node/src/dpos.rs`.

use crate::{
    application::{
        derive_with_visibility_retry, BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder,
        ExecutedChain, OrderingAssembler,
    },
    cold_start_jump::ElSync as _,
    order_block::{anchor_order_block, K},
    scheme::epoch_committee_from_snapshot,
    slasher::actor::{SharedCacheFallback, SlasherTxSink, StaleEpochFallback},
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
use fluentbase_staking_reader::{
    reader::{StakingReaderConfig, ValidatorSetSnapshot},
    EpochTransition, RethStakingStateReader, TransitionOutcome, ValidatorSetCache,
};
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
        // Mirror the live executor's 3-way seed resolution (executor::SeedLookup):
        // a PRESENT finalization with no seed is a genuine no-beacon epoch → the
        // agreed `order.digest()` fallback (seed = None); a present finalization
        // WITH a seed → use it. But a MISSING or unreadable finalization at a
        // height we ourselves finalized must NOT silently fall back to seedless
        // derive — on a beacon-active chain that re-rolls prev_randao and forks
        // the restart (the same asymmetry the fail-loud `order` lookup above
        // enforces). Fail loud instead.
        let fin = finalizations
            .get(Identifier::Index(h))
            .await
            .map_err(|e| eyre!("reading marshal finalizations at height {h}: {e}"))?
            .ok_or_else(|| {
                eyre!(
                    "crash-survivor recovery: marshal finalizations has no cert at height {h} \
                     — cannot recover the beacon seed without diverging prev_randao; \
                     re-sync the EL disk from a snapshot"
                )
            })?;
        let seed = fin
            .certificate
            .seed()
            .map(|signature| crate::beacon::seed::Seed {
                target_round: fin.proposal.round,
                signature,
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
pub struct DposLayerConfig<D, XC, A, U> {
    pub bls_keypair: ValidatorBlsKeypair,
    pub peer_keypair: commonware_cryptography::ed25519::PrivateKey,
    pub slasher_sink: Arc<dyn SlasherTxSink>,
    pub staking_config: StakingReaderConfig,
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
    /// signer engine is a CONSUMER of its shared `ceremony_store` (the C-gate
    /// share source + the proposer's `beacon_outcome`) and `committee_for`, re-uses
    /// its `oracle` (the single network's peer set) + its already-registered
    /// `beacon_metrics`, and CLONES its 5 `MuxHandle`s + `subscribe()`s the vote
    /// backup to wire the OuterEngine's per-promotion sub-channels — it never
    /// re-builds the network, re-spawns the `DkgActor`, re-registers the metrics, or
    /// re-binds `listen`.
    pub beacon_plane: SharedBeaconPlane,
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
    /// The shared live-DKG store: written by the always-on `DkgActor`, read by the
    /// per-engine `BeaconVerify` (C gate + propose) and `beacon_resolver` (sign).
    pub ceremony_store: crate::beacon::actor::CeremonyStore,
    /// Edge-trigger the `DkgActor` fires (`notify_one`) when a share lands in
    /// `ceremony_store`, so `EpochManager::run` wakes the instant its share is
    /// memoized instead of polling. Same `Arc` held by the actor (the producer) and
    /// the manager (the single consumer).
    pub share_notify: Arc<tokio::sync::Notify>,
    /// On-chain committee resolver (the deal/carry-forward set), built once over the
    /// persistent reader+provider and shared with both the actor and the verify gate.
    pub committee_for: crate::beacon::actor::CommitteeFor,
    /// Beacon counters, registered ONCE at the persistent layer; cloned (never
    /// re-registered) into the executor + each per-epoch engine.
    pub beacon_metrics: crate::beacon::metrics::BeaconMetrics,
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
        return Err(eyre!(
            "EL is past epoch 0 (finalized {cs_finalized} >= activation {activation} + interval \
             {interval}) with an empty consensus archive; refusing to anchor DPoS on a state of \
             unknown provenance. Configure --dpos.follower-upstream so the node cert-follows and \
             cold-start-jumps to the verified frontier, or restore the consensus archive."
        ));
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

/// Cold-start adapter for [`crate::cold_start_jump::cold_start_jump`]'s typed
/// terminal [`JumpOutcome`]: maps it back to the `Result<Option<(landing, hash,
/// floor)>>` the two single-shot pre-engine call sites consume.
///
/// The cold-start path is SINGLE-SHOT — it has no `Update::Tip` to retry on — so
/// it DELIBERATELY re-fuses a `Stalled` transport error to `Err` (preserving the
/// pre-existing `?`-abort behaviour), exactly like an `AuthFailed`. Only the
/// steady-state executor spawn treats `Stalled` as non-fatal (§9.6). `Landed` ⇒
/// `Ok(Some(..))`, `Lagging` ⇒ `Ok(None)`.
fn jump_landing_or_abort(
    outcome: crate::cold_start_jump::JumpOutcome,
) -> eyre::Result<Option<(u64, B256, u64)>> {
    use crate::cold_start_jump::JumpOutcome;
    match outcome {
        JumpOutcome::Landed {
            landing,
            hash,
            floor,
        } => Ok(Some((landing, hash, floor))),
        JumpOutcome::Lagging => Ok(None),
        // Single-shot cold-start: a transport stall is re-fused to fatal exactly
        // like a forged-branch rejection (see the doc above).
        JumpOutcome::Stalled(e) | JumpOutcome::AuthFailed(e) => Err(e),
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
            staking_config,
            upstream,
            deriver,
            executed,
            assembler,
            fee_recipient,
            target_gas_limit,
            feed,
            spawn_unblocked,
            beacon_plane,
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
        } = cfg;

        // The plane owns the 5 persistent Muxers; this promotion CLONES their handles
        // (to register fresh per-epoch / subchannel-0 routes against the SAME broker
        // tasks) and `subscribe()`s a fresh vote-backup receiver. No raw move-only
        // halves are consumed here, so a later demote→re-promote re-clones cleanly.
        let SharedBeaconPlane {
            oracle,
            ceremony_store,
            share_notify,
            committee_for,
            beacon_metrics,
            vote_mux,
            cert_mux,
            resolver_mux,
            broadcast_mux,
            marshal_mux,
            vote_backup,
        } = beacon_plane;
        let vote_backup_rx = vote_backup.subscribe().await;

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
        )?;
        let (mut latest_finalized, mut latest_finalized_hash) = if kind
            == ColdStartKind::FreshMigration
        {
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
                    genesis_hash,
                    dpos_activation_block,
                );
                // The post-sync cert `verify()` inside `verify_jump_authenticated`
                // needs a `&mut CryptoRngCore`; clone `ctx` (a cheap handle) so the
                // move does not consume the launcher's own `ctx`.
                let mut jump_ctx = ctx.clone();
                if let Some((h, hash, floor)) = jump_landing_or_abort(
                    crate::cold_start_jump::cold_start_jump(
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
                    .await,
                )? {
                    latest_finalized = h;
                    latest_finalized_hash = hash;
                    jumped_marshal_floor = Some(Height::new(floor));
                }
            }
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
            cache,
            oracle.clone(),
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
        let genesis_block = anchor_order_block(&genesis_sealed);

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
        let shutdown_for_hook = shutdown.clone();
        let et_for_hook = et_arc.clone();
        let ctx_for_hook = ctx.with_label("boundary_hook");
        let errors_for_hook = consecutive_errors.clone();
        let boundary_hook: Arc<dyn Fn(crate::order_block::OrderBlock) + Send + Sync> =
            Arc::new(move |block: crate::order_block::OrderBlock| {
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

        // The live-DKG `ceremony_store` (written by the always-on `DkgActor`) and the
        // `committee_for` resolver arrive SHARED from the persistent beacon plane
        // (node crate) via [`SharedBeaconPlane`]; this signer engine is a read-only
        // consumer (the C gate + propose `beacon_outcome` via `BeaconVerify`, and the
        // per-epoch signing material via `beacon_resolver`). The disk reload of
        // `<datadir>/beacon/` happened ONCE at the plane's startup — not here.

        // Beacon is MANDATORY under `--dpos` (always-on live DKG — no opt-out). The
        // verify/propose context reads the SHARED `ceremony_store` (written by the
        // always-on `DkgActor` in the persistent plane) and the SHARED on-chain
        // `committee_for`: the boundary "C" gate reads this node's memoized
        // (PK_E, share) and the proposer asserts PK_E in `beacon_outcome`. The DKG
        // actor + beacon p2p halves are NOT (re)built here — they live in the plane.
        let beacon_verify = {
            let beacon_for_epoch: crate::application::BeaconForEpoch = {
                let store = ceremony_store.clone();
                Arc::new(move |epoch: u64| store.read().ok().and_then(|m| m.get(&epoch).cloned()))
            };
            let bv = crate::application::BeaconVerify::new(
                beacon_for_epoch,
                committee_for.clone(),
                dpos_activation_block,
                interval.into(),
            );
            // DEVNET/TEST-ONLY: thread the byzantine mode into the verify/propose
            // beacon context (no-op + field-absent in a production build).
            #[cfg(feature = "dpos-devnet-byzantine")]
            let bv = bv.with_byzantine(byzantine);
            Some(bv)
        };

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

        // Per-epoch threshold beacon resolver for the combined consensus scheme:
        // each vote carries the seed partial (round-keyed), so the seed is
        // recovered from the notarization/finalization certificate — no separate
        // seed plane. The key ROTATES per epoch: the live-DKG `ceremony_store`
        // holds (PK_E, share) for every epoch this node was a member of where the
        // committee changed; `resolve(E)` returns the most-recent such key at or
        // before E (carry-forward across stable epochs). There is NO genesis-baked
        // fallback key anymore (the genesis deal was removed): epoch 1 is seedless
        // and the first key is the deterministic epoch-2 live DKG, so the resolver
        // bottoms out at `None` (a keyless epoch ⇒ pure-multisig / gated fallback).
        let beacon_namespace = crate::beacon::seed::seed_namespace(&fluent_namespace(chain_id));
        let genesis_beacon_key: Option<(
            commonware_cryptography::bls12381::primitives::sharing::Sharing<
                commonware_cryptography::bls12381::primitives::variant::MinSig,
            >,
            Option<commonware_cryptography::bls12381::primitives::group::Share>,
            Vec<u8>,
        )> = None;

        // Per-epoch threshold beacon resolver for the combined consensus scheme:
        // each vote carries the seed partial (round-keyed), so the seed is
        // recovered from the notarization/finalization certificate — no separate
        // seed plane. The key ROTATES per epoch: the live-DKG `ceremony_store`
        // holds (PK_E, share) for every epoch this node was a member of where the
        // committee changed; `resolve(E)` returns the most-recent such key at or
        // before E (carry-forward across stable epochs), falling back to the
        // genesis bootstrap key (epoch 0 / pre-first-rotation).
        //
        // The on-chain getEpochBeaconKey carry-forward cross-check is GONE (the
        // PK_E layer was removed). A stored key is carried forward to E
        // unconditionally; a stale carried share (committee changed at E) emits
        // seed partials that peers reject via `verify_seed_partial` (the vote
        // Nullifies → does not count toward quorum), and the certify hook gates the
        // boundary, so a wrong carry-forward can NEVER finalize a bad seed. The
        // dropped guard was only an early-demote LIVENESS optimization (a
        // missed-ceremony member now signs doomed partials instead of demoting to
        // cert-follow); fresh-share committee[E] members still form the quorum.
        let beacon_resolver: crate::epoch_manager::BeaconResolver = {
            let store = ceremony_store.clone();
            let namespace = beacon_namespace;
            let genesis = genesis_beacon_key;
            Arc::new(move |epoch: u64| {
                if let Ok(m) = store.read() {
                    if let Some((_stored_epoch, (out, share))) = m.range(..=epoch).next_back() {
                        // Exact-epoch hit OR carry-forward: return the local share.
                        return Some((
                            out.public().clone(),
                            Some(share.clone()),
                            namespace.clone(),
                        ));
                    }
                }
                genesis.clone()
            })
        };

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
        // executor runs. Enabled wherever an upstream is configured (a follower OR
        // a validator-with-upstream); `None` for a plain validator (it catches up
        // on the consensus-plane treadmill). The executor runs it synchronously in
        // its `select!` arm, so its `sync_to` FCU is serialized with every other
        // reth write the executor makes — the executor stays the sole reth writer.
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let up = up.clone();
            let provider = provider.clone();
            let evm_config = evm_config.clone();
            let staking_config = staking_config.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let ctx = ctx.clone();
            let cb: crate::executor::ReJump = Arc::new(move |from: u64| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let beacon_engine_handle = beacon_engine_handle.clone();
                // `verify_jump_authenticated` needs a `&mut (Clock + CryptoRngCore)`;
                // a fresh clone per call so the closure stays re-usable.
                let mut jump_ctx = ctx.clone();
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
                    let el = crate::cold_start_jump::RethElSync::new(
                        jump_ctx.clone(),
                        provider.clone(),
                        beacon_engine_handle,
                        genesis_hash,
                        dpos_activation_block,
                    );
                    // Return the typed terminal `JumpOutcome` verbatim — the
                    // executor's completion arm classifies it (Landed re-seeds;
                    // Stalled is NON-fatal + retried on the next Tip; AuthFailed
                    // is fail-closed). §9.6.
                    crate::cold_start_jump::cold_start_jump(
                        from,
                        &up,
                        &committees,
                        &el,
                        // No L1 checkpoint on the validator path (trustless
                        // POST-sync committee read at the landing).
                        None,
                        dpos_activation_block,
                        &mut jump_ctx,
                    )
                    .await
                }) as futures::future::BoxFuture<'static, _>
            });
            cb
        });

        let outer = OuterBuilder {
            me: me.clone(),
            blocker: oracle.clone(),
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block,
            signer_keypair: Some(bls_keypair),
            beacon_resolver,
            beacon_share_notify: share_notify,
            spawn_unblocked,
            re_jump,
            soft_enter_committees,
            beacon_metrics,
            beacon_verify,
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

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
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
        } = reth;
        let FollowerLayerConfig {
            me,
            staking_config,
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
                genesis_hash,
                activation,
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
                                let hash =
                                    wait_for_activation_block(&ctx, &provider, activation).await?;
                                (activation, hash)
                            }
                        },
                        None => {
                            let hash =
                                wait_for_activation_block(&ctx, &provider, activation).await?;
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
                    let hash = provider
                        .block_hash(h)?
                        .ok_or_else(|| eyre!("reth does not hold the clamped landing {h}"))?;
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
            if let Some((h, hash, floor)) = jump_landing_or_abort(
                crate::cold_start_jump::cold_start_jump(
                    anchor_height,
                    up,
                    &committees,
                    &el,
                    l1_checkpoint_hash,
                    activation,
                    &mut jump_ctx,
                )
                .await,
            )? {
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
        let finalized_hash = provider
            .block_hash(finalized_floor)?
            .ok_or_else(|| eyre!("reth does not hold the finality floor {finalized_floor}"))?;
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
        let genesis_unsealed = provider
            .block_by_number(anchor_height)
            .map_err(|e| eyre!("follower genesis block read at height {anchor_height} failed: {e}"))?
            .ok_or_else(|| {
                eyre!("follower genesis block missing from MDBX at height {anchor_height}")
            })?;
        let genesis_sealed: SealedBlock<RethBlock> = SealedBlock::seal_slow(genesis_unsealed);
        let genesis_block = anchor_order_block(&genesis_sealed);

        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head block number at startup")?;
        let head_info = canonical_state.chain_info();

        let epoch_length_blocks =
            NonZeroU64::new(interval as u64).ok_or_eyre("epoch_block_interval must be > 0")?;

        // A follower has NO beacon plane: no DKG share, no live-DKG store. The
        // resolver always returns `None` (a keyless / verify-only node), and there
        // is no `beacon_verify` propose/gate (it never proposes).
        let beacon_resolver: crate::epoch_manager::BeaconResolver = Arc::new(|_| None);
        let beacon_metrics = crate::beacon::metrics::BeaconMetrics::default();
        beacon_metrics.register(&ctx);

        // Slasher reader + fallback are required by the OuterBuilder type but the
        // slasher is never STARTED on a follower (`run_follower` drops the unstarted
        // actor) — a non-signer never submits slashing.
        let slasher_reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );
        let cache = Arc::new(Mutex::new(
            ValidatorSetCache::init(ctx.with_label("follower_slasher_cache"))
                .await
                .wrap_err("failed initializing follower slasher ValidatorSetCache")?,
        ));
        let slasher_stale_fallback: Arc<dyn StaleEpochFallback> =
            Arc::new(SharedCacheFallback(cache));
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
                // Read each committee at the current finalized hash (committee[E]
                // is content-invariant across any in-epoch executed hash). Truncate
                // at the first missed/unreadable committee → a contiguous prefix.
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
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let up = up.clone();
            let provider = provider.clone();
            let evm_config = evm_config.clone();
            let staking_config = staking_config.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let ctx = ctx.clone();
            let cb: crate::executor::ReJump = Arc::new(move |from: u64| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let beacon_engine_handle = beacon_engine_handle.clone();
                let mut jump_ctx = ctx.clone();
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
                    let el = crate::cold_start_jump::RethElSync::new(
                        jump_ctx.clone(),
                        provider.clone(),
                        beacon_engine_handle,
                        genesis_hash,
                        activation,
                    );
                    // Return the typed terminal `JumpOutcome` verbatim — the
                    // executor's completion arm classifies it (§9.6).
                    crate::cold_start_jump::cold_start_jump(
                        from,
                        &up,
                        &committees,
                        &el,
                        l1_checkpoint_hash,
                        activation,
                        &mut jump_ctx,
                    )
                    .await
                }) as futures::future::BoxFuture<'static, _>
            });
            cb
        });

        let outer = OuterBuilder {
            me: me.clone(),
            blocker: oracle.clone(),
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block: activation,
            signer_keypair: None,
            beacon_resolver,
            beacon_share_notify: Arc::new(tokio::sync::Notify::new()),
            spawn_unblocked: Arc::new(tokio::sync::Notify::new()),
            re_jump,
            soft_enter_committees,
            beacon_metrics,
            beacon_verify: None,
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
            boundary_hook: Arc::new(|_| {}),

            last_execution_finalized_height,
            initial_finalized: (Height::new(anchor_height), anchor_hash),
            initial_head: (Height::new(head_info.best_number), head_info.best_hash),
            marshal_floor: Some(
                jumped_marshal_floor.unwrap_or_else(|| Height::new(finalized_floor)),
            ),
            fcu_heartbeat_interval,
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_config.staking_address,
            slasher_reader,
            slasher_latest_finalized_hash,
            slasher_stale_fallback,
            slasher_sink: Arc::new(NoopSlasherSink),
            slasher_wal_partition: "slasher-wal".into(),

            feed,

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine: None,
        }
        .build(ctx.with_label("outer_engine"))
        .await?;

        // Register the initial epoch's verify-only scheme so the marshal can
        // verify the inlet's certs from cold-start (before any boundary fires).
        let committees_src = crate::cert_inlet::RethCommitteeSource::new(
            RethStakingStateReader::new(provider.clone(), evm_config.clone(), staking_config.clone()),
            chain_id,
            {
                let p = provider.clone();
                Arc::new(move || {
                    let n = p.finalized_block_number().ok()??;
                    p.block_hash(n).ok().flatten()
                })
            },
        );
        if let Ok(scheme) =
            crate::cert_inlet::CommitteeSource::scheme_at(&committees_src, initial_epoch_u64, anchor_hash)
        {
            outer.cold_start_register(Epoch::new(initial_epoch_u64), scheme);
        }

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
        let consensus_handle =
            outer.start_follower(broadcast_mux, ctx.clone(), upstream.clone());

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
        let inlet_committees = crate::cert_inlet::RethCommitteeSource::new(
            RethStakingStateReader::new(provider.clone(), evm_config.clone(), staking_config.clone()),
            chain_id,
            {
                let p = provider.clone();
                let live_frontier = live_frontier.clone();
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
                    // Fall back to the finalized hash if reth has not yet imported
                    // the cursor block (the cert can land a beat before EL-sync).
                    p.block_hash(read_at)
                        .ok()
                        .flatten()
                        .or_else(|| p.block_hash(fin).ok().flatten())
                })
            },
        );
        let shutdown_for_inlet = shutdown.clone();
        // DATA-fault rotation trigger (#7): after MAX_UPSTREAM_FAULTS consecutive
        // unverifiable certs over a healthy connection the inlet rotates to the
        // next configured upstream URL (connection-level failover can never see a
        // bad PAYLOAD on a live connection). Built from a clone of the SAME
        // `CertUpstream` handle the keepalive holds; `rotate()` drops the
        // connection so the WS actor's run loop advances to the next URL.
        let inlet_rotate: Option<crate::cert_inlet::RotateUpstream> =
            upstream.as_ref().map(crate::cert_follow::CertUpstream::rotate_callback);
        drop(ctx.with_label("cert_inlet").spawn(move |c| async move {
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
            let mut inlet = crate::cert_inlet::CertInlet::new(
                inlet_marshal,
                inlet_committees,
                c,
            )
            .with_epoch_math(activation, interval)
            .with_tee(crate::cert_inlet::LiveFrontierTee {
                live_height: live_frontier,
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
        }));

        Ok(DposLayerHandle {
            consensus_handle,
            cert_mailbox,
        })
    }
}

#[cfg(test)]
mod cold_start_kind_tests {
    use super::{cold_start_jump_eligible, jump_landing_or_abort, resolve_cold_start_kind, ColdStartKind};
    use crate::cold_start_jump::JumpOutcome;
    use alloy_primitives::B256;

    const ACTIVATION: u64 = 192;
    const INTERVAL: u32 = 64;

    // The single-shot cold-start adapter's mapping (the steady-state classifier
    // is tested in cold_start_jump.rs; this pins the FATAL re-fuse the cold-start
    // path deliberately applies — see `sync_to_stall_is_classified_stalled`).
    #[test]
    fn jump_landing_or_abort_maps_each_outcome() {
        let hash = B256::repeat_byte(0x5A);
        assert_eq!(
            jump_landing_or_abort(JumpOutcome::Landed { landing: 700, hash, floor: 697 })
                .expect("Landed is Ok"),
            Some((700, hash, 697)),
            "Landed ⇒ Ok(Some(landing, hash, floor))"
        );
        assert_eq!(
            jump_landing_or_abort(JumpOutcome::Lagging).expect("Lagging is Ok"),
            None,
            "Lagging ⇒ Ok(None) (no-op; inlet pulls cover the residual gap)"
        );
        assert!(
            jump_landing_or_abort(JumpOutcome::Stalled(eyre::eyre!("transport stall"))).is_err(),
            "Stalled ⇒ Err (single-shot cold-start re-fuses a transport stall to fatal)"
        );
        assert!(
            jump_landing_or_abort(JumpOutcome::AuthFailed(eyre::eyre!("forged branch"))).is_err(),
            "AuthFailed ⇒ Err (forged/unagreed branch; fail closed)"
        );
    }

    #[test]
    fn fresh_migration_never_jumps_even_with_an_upstream() {
        // The load-bearing #7 guard: a FreshMigration MUST anchor at
        // dposActivationBlock (clean-halt invariant), so the cold-start jump is
        // inert for it regardless of whether an upstream is configured.
        assert!(!cold_start_jump_eligible(ColdStartKind::FreshMigration, true));
        assert!(!cold_start_jump_eligible(ColdStartKind::FreshMigration, false));
    }

    #[test]
    fn restart_jumps_only_with_an_upstream() {
        // A no-upstream validator catches up on the consensus-plane treadmill,
        // NOT via the jump (Risk-1).
        assert!(cold_start_jump_eligible(ColdStartKind::Restart, true));
        assert!(!cold_start_jump_eligible(ColdStartKind::Restart, false));
    }

    #[test]
    fn overshoot_with_empty_archive_is_fatal_pointing_at_follower_upstream() {
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

    #[test]
    fn zero_activation_is_the_unscheduled_sentinel_and_fatal() {
        let err = resolve_cold_start_kind(0, 0, INTERVAL, 0).unwrap_err();
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
