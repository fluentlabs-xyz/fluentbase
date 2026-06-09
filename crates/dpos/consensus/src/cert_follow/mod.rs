//! Trustless cert-follower engine (`--cert-follow`).
//!
//! A non-validator node that **verifies, not trusts**: it pulls finality
//! certificates (2f+1 BLS multisig) from an upstream; the driver verifies every
//! forward cert against the on-chain epoch committee (`EpochSchemeProvider`,
//! `Finalization::verify`) before the marshal finalizes, and the executor drives
//! the follower's own reth from the verified finalized stream. The cold-start
//! anchor is authenticated transitively (the first verified descendant cert
//! commits the anchor's hash via `parent_hash`); its *hash source* is the
//! remaining trust input, deferred to the L1 anchor source.
//! Reuses the entire
//! validator marshal + executor + epoch-rotation stack; the only follower-only
//! pieces are the upstream seam, the gap-repair resolver, the sync driver, and
//! three glue stubs.
//!
//! Mirrors tempo `follow/engine.rs`. The crate split (consensus vs node) means
//! the WS transport lives node-side behind the [`CertUpstream`] trait; this
//! module is transport-agnostic.

mod driver;
mod resolver;
mod stubs;
mod upstream;

use crate::{
    application::BeaconEngineLike,
    block::Block,
    dpos::{
        derive_cold_start_heights, read_consensus_archive_last_finalized, wait_for_activation_block,
    },
    executor,
    outer::{follower_marshal_config, init_finalizations_archive, EpochSchemeProvider},
    scheme::epoch_committee_from_snapshot,
    OriginEpocher,
};
use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::{
    marshal::{core::Actor as MarshalActor, Update},
    types::{Epoch, Height},
    Heightable as _, Reporters,
};
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use eyre::{eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::{fluent_namespace, scheme::build_verifier};
use fluentbase_staking_reader::{
    reader::{epoch_of_block, StakingReaderConfig},
    EpochTransition, RethStakingStateReader, ValidatorSetCache,
};
use reth_chain_state::CanonicalInMemoryState;
use reth_ethereum_engine_primitives::EthPayloadAttributes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
use reth_primitives_traits::SealedBlock;
use reth_storage_api::{
    BlockHashReader, BlockNumReader, BlockReader, HeaderProvider, StateProviderFactory,
};
use std::{num::NonZeroU64, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
pub use upstream::{CertUpstream, UpstreamFinalized};

/// Reth handles the follower engine needs. Assembled node-side from the reth
/// `FullNode`. Unlike the validator [`RethHandle`](crate::dpos::RethHandle) there
/// is no payload builder — a follower never proposes blocks.
pub struct CertFollowRethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    pub canonical_state: CanonicalInMemoryState<EthPrimitives>,
    pub genesis_hash: B256,
}

/// Operator-supplied follower configuration.
pub struct CertFollowConfig {
    pub staking_config: StakingReaderConfig,
    pub partition_prefix: String,
    pub mailbox_size: usize,
    pub fcu_heartbeat_interval: Duration,
    pub fcu_pace: Duration,
}

/// Handle the host adapter supervises alongside its WS-upstream actor.
pub struct CertFollowHandle {
    pub consensus_handle: Handle<()>,
}

/// Namespace type for the launch entry point.
pub struct CertFollowLayer;

impl CertFollowLayer {
    /// Launch the trustless cert-follower: read the reth cold-start anchor,
    /// register the anchor epoch's BLS verifier, stand up marshal + executor +
    /// resolver + driver + the epoch-rotation forwarder, and start them under an
    /// internal supervisor. `upstream` serves the resolver's by-height pulls;
    /// `finalized_rx` is the live finalized-cert stream the node's WS actor pushes.
    pub async fn launch<Provider, EvmConfig, BeaconEngine, U>(
        ctx: Context,
        reth: CertFollowRethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: CertFollowConfig,
        upstream: U,
        finalized_rx: mpsc::UnboundedReceiver<UpstreamFinalized>,
        shutdown: CancellationToken,
    ) -> eyre::Result<CertFollowHandle>
    where
        Provider: BlockReader<Block = RethBlock>
            + BlockHashReader
            + BlockNumReader
            + StateProviderFactory
            + HeaderProvider<Header = Header>
            + Clone
            + Send
            + Sync
            + 'static,
        EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
        BeaconEngine: BeaconEngineLike<
                PayloadAttrs = EthPayloadAttributes,
                ExecutionData = SealedBlock<RethBlock>,
            > + Clone
            + Send
            + Sync
            + 'static,
        U: CertUpstream,
    {
        let CertFollowRethHandle {
            provider,
            evm_config,
            beacon_engine_handle,
            chain_id,
            canonical_state,
            genesis_hash,
        } = reth;
        let CertFollowConfig {
            staking_config,
            partition_prefix,
            mailbox_size,
            fcu_heartbeat_interval,
            fcu_pace,
        } = cfg;

        // Epoch geometry. `ChainConfig` lives in genesis state (present from block
        // 0), so reading at reth's current finalized-or-genesis hash is always valid.
        let (_rf_num, rf_hash, _h0_num, _h0_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);
        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );
        let dpos_activation_block = reader.dpos_activation_block(rf_hash)?;
        let interval = reader.epoch_block_interval(rf_hash)?;
        let epoch_length_blocks =
            NonZeroU64::new(interval as u64).ok_or_eyre("epoch_block_interval must be > 0")?;

        // Cold-start anchor. A fresh follower (empty marshal archive) anchors at the
        // DPoS **activation block**, NOT genesis: DPoS finality certs begin at
        // activation and epoch 0's committee is committed by then, whereas
        // pre-activation blocks are Tempo-era with no certs (gap-repairing below
        // activation would never resolve). A restart resumes at the marshal
        // archive's last finalized. reth devp2p-syncs the chain up to the anchor;
        // `wait_for_activation_block` blocks until it holds the block + derives the
        // local-canonical hash.
        let archive_finalized =
            read_consensus_archive_last_finalized(&ctx, &partition_prefix).await?;
        let anchor_height = if archive_finalized <= dpos_activation_block {
            dpos_activation_block
        } else {
            archive_finalized
        };

        // EL-sync bootstrap. A fresh follower's reth is empty; post-merge reth won't
        // sync historical blocks without an Engine-API head. If reth lacks the
        // anchor, fetch the upstream's latest finalized head and FCU reth to it,
        // triggering devp2p backward-sync from the trusted peer. Weak-subjectivity:
        // ONLY the EL-sync head hash is trusted — reth validates every block (state
        // root) and the driver verifies every live cert from the anchor forward (the
        // first verified cert authenticates the anchor's hash via its `parent_hash`).
        if provider
            .block_hash(anchor_height)
            .wrap_err("block_hash(anchor) probe failed")?
            .is_none()
        {
            match upstream.get_latest().await {
                Some(latest) => {
                    let latest_block = latest.block;
                    let latest_hash = latest_block.block_hash();
                    let latest_height = latest_block.height().get();
                    info!(
                        latest_height,
                        anchor_height, "cert-follow: driving reth EL-sync to upstream checkpoint"
                    );
                    let _ = beacon_engine_handle
                        .new_payload(latest_block.into_inner())
                        .await;
                    let _ = beacon_engine_handle
                        .fork_choice_updated(
                            ForkchoiceState {
                                head_block_hash: latest_hash,
                                safe_block_hash: latest_hash,
                                finalized_block_hash: genesis_hash,
                            },
                            None,
                        )
                        .await;
                }
                None => warn!(
                    "cert-follow: upstream getLatest returned none; relying on existing reth state"
                ),
            }

            // Wait (bounded) for reth's backward-sync to canonicalize the anchor
            // (block_hash present ⇒ executed ⇒ committee state queryable).
            const EL_SYNC_WAIT: Duration = Duration::from_secs(120);
            let deadline = ctx.current() + EL_SYNC_WAIT;
            while provider
                .block_hash(anchor_height)
                .wrap_err("block_hash(anchor) probe during EL-sync")?
                .is_none()
            {
                if ctx.current() >= deadline {
                    return Err(eyre!(
                        "cert-follow: reth did not EL-sync to anchor {anchor_height} within \
                         {EL_SYNC_WAIT:?} — is the trusted peer serving block bodies?"
                    ));
                }
                ctx.sleep(Duration::from_secs(2)).await;
            }
        }

        let anchor_hash = wait_for_activation_block(&ctx, &provider, anchor_height).await?;

        // reth's actual canonical head (devp2p-synced, ahead of the consensus
        // anchor). Seed the executor head here so it never issues a backward FCU to
        // the anchor (reth spec-skips a backward FCU to an ancestor → wedge);
        // finalized advances forward from the anchor as certs verify.
        let chain = canonical_state.chain_info();
        let (head_num, head_hash) = (chain.best_number, chain.best_hash);
        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head at follower startup")?;

        let initial_epoch_u64 = epoch_of_block(anchor_height, interval, dpos_activation_block);
        let initial_snapshot = reader.epoch_committee_snapshot(initial_epoch_u64, anchor_hash)?;
        if initial_snapshot.validators.is_empty() {
            return Err(eyre!(
                "staking contract returned empty committee for epoch {initial_epoch_u64} \
                 (read at anchor block {anchor_height}); point --cert-follow at a network \
                 whose committee is committed, or wait for reth to sync past DPoS activation"
            ));
        }
        info!(
            chain_id,
            anchor_height,
            head_num,
            initial_epoch = initial_epoch_u64,
            interval,
            "cert-follower cold-start anchor resolved"
        );

        // Marshal: archives (shared format with the validator) + scheme provider
        // + epocher.
        let page_cache = crate::outer::new_page_cache(&ctx);
        let finalizations_by_height =
            init_finalizations_archive(&ctx, &partition_prefix, page_cache.clone()).await;
        let finalized_blocks =
            crate::outer::init_finalized_blocks_archive(&ctx, &partition_prefix).await;
        let scheme_provider = EpochSchemeProvider::new();
        let epocher = OriginEpocher::new(dpos_activation_block, epoch_length_blocks);
        let (marshal, marshal_mailbox, last_consensus_finalized_height) = MarshalActor::init(
            ctx.with_label("marshal"),
            finalizations_by_height,
            finalized_blocks,
            follower_marshal_config(
                partition_prefix.clone(),
                mailbox_size,
                page_cache,
                scheme_provider.clone(),
                epocher.clone(),
            ),
        )
        .await;
        // Pin the in-order floor to the anchor: dispatch from anchor+1 forward
        // (raises-only, so a restart with a higher archive floor is a no-op).
        marshal_mailbox.set_floor(Height::new(anchor_height)).await;

        // Register the anchor epoch's verifier BEFORE any cert is processed —
        // otherwise the marshal would reject the first finalization (no scheme).
        let namespace = fluent_namespace(chain_id);
        let initial_committee = epoch_committee_from_snapshot(&initial_snapshot)
            .map_err(|e| eyre!("anchor committee has non-unique participants: {e:?}"))?;
        scheme_provider.register(
            Epoch::new(initial_epoch_u64),
            build_verifier(&namespace, initial_committee.bimap),
        );

        // The anchor itself carries no separately-verifiable cert: in a Tempo→DPoS
        // migration the anchor (`dposActivationBlock`) is the switchover block,
        // finalized by Tempo, so the upstream's DPoS marshal has no finalization for
        // it (the first DPoS cert is anchor+1). Authentication is transitive instead:
        // the driver BLS-verifies every forward cert, and the consensus digest IS the
        // EVM block hash committing `parent_hash`, so the first verified descendant
        // cert authenticates the anchor's hash. The anchor *hash* source (the trusted
        // EL-sync head) is the remaining trust input — closed by the deferred L1
        // anchor source, not by verifying a non-existent anchor cert.

        // Executor — drives reth from the verified finalized stream.
        let (executor_actor, executor_mailbox) = executor::Actor::init(
            ctx.with_label("executor"),
            executor::Config {
                beacon_engine: beacon_engine_handle,
                marshal: marshal_mailbox.clone(),
                fcu_heartbeat_interval,
                last_consensus_finalized_height,
                last_execution_finalized_height,
                initial_finalized: (Height::new(anchor_height), anchor_hash),
                initial_head: (Height::new(head_num), head_hash),
                fcu_pace,
                canonical_state,
            },
        );

        // Epoch rotation: observer-only EpochTransition (no peer tracking) whose
        // boundary_tx surfaces each new epoch's frozen committee; a forwarder turns
        // that into a BLS verifier and registers it so the marshal can verify the
        // next epoch's certs.
        let cache = Arc::new(Mutex::new(
            ValidatorSetCache::init(ctx.with_label("follower_cache"))
                .await
                .wrap_err("failed initializing follower ValidatorSetCache")?,
        ));
        let (boundary_tx, mut boundary_rx) =
            mpsc::channel::<(u64, fluentbase_staking_reader::reader::ValidatorSetSnapshot)>(64);
        let reader_for_et =
            RethStakingStateReader::new(provider.clone(), evm_config, staking_config);
        let mut epoch_transition = EpochTransition::new(
            reader_for_et,
            cache,
            stubs::NullPeerSetSink,
            fluentbase_p2p::constants::MAX_PEER_SET_SIZE as usize,
            Some(boundary_tx),
        );

        // Scheme-registration forwarder. Spawned BEFORE cold_start so the
        // cold_start boundary fire is drained (re-registering the anchor scheme is
        // idempotent — EpochSchemeProvider::register is insert-or-equal).
        let scheme_provider_for_fwd = scheme_provider.clone();
        let namespace_for_fwd = namespace.clone();
        drop(
            ctx.with_label("scheme_forwarder")
                .spawn(move |_| async move {
                    while let Some((epoch, snap)) = boundary_rx.recv().await {
                        match epoch_committee_from_snapshot(&snap) {
                            Ok(committee) => {
                                scheme_provider_for_fwd.register(
                                    Epoch::new(epoch),
                                    build_verifier(&namespace_for_fwd, committee.bimap),
                                );
                                info!(epoch, "cert-follower registered next epoch verifier");
                            }
                            Err(e) => error!(
                                epoch,
                                ?e,
                                "cert-follower: cannot build verifier from committee snapshot"
                            ),
                        }
                    }
                }),
        );
        // Seed last_tracked_epoch so on_finalized boundary detection works.
        epoch_transition
            .cold_start(anchor_hash, anchor_height)
            .await
            .wrap_err("follower epoch_transition cold_start failed")?;

        // Driver + resolver + null broadcast.
        let (driver_actor, marshal_reporter) = driver::try_init(
            ctx.with_label("driver"),
            marshal_mailbox.clone(),
            scheme_provider.clone(),
            epocher,
            finalized_rx,
            epoch_transition,
        );
        let (resolver_actor, resolver_mailbox, resolver_rx) = resolver::try_init(
            ctx.with_label("resolver"),
            resolver::Config {
                provider,
                upstream,
                mailbox_size,
            },
        );
        let broadcast = stubs::null_broadcast(ctx.with_label("broadcast"), mailbox_size);
        let app_reporter = stubs::AppReporter::new(executor_mailbox);

        // Start the actors. The marshal fans Update<Block> to BOTH the app
        // reporter (executor) and the driver (epoch rotation) via Reporters.
        let driver_handle = driver_actor.start();
        let executor_handle = executor_actor.start();
        let resolver_handle = resolver_actor.start();
        let marshal_handle = marshal.start(
            Reporters::<Update<Block>, stubs::AppReporter, driver::MarshalReporter>::from((
                app_reporter,
                marshal_reporter,
            )),
            broadcast,
            (resolver_rx, resolver_mailbox),
        );

        // Internal supervisor: on first subsystem exit, abort the rest + cancel
        // the shared shutdown so the host brings the node down gracefully.
        let consensus_handle = ctx
            .with_label("follower_supervisor")
            .spawn(move |_| async move {
                let mut marshal_handle = marshal_handle;
                let mut executor_handle = executor_handle;
                let mut driver_handle = driver_handle;
                let mut resolver_handle = resolver_handle;
                let exit = tokio::select! {
                    r = &mut marshal_handle => ("marshal", r),
                    r = &mut executor_handle => ("executor", r),
                    r = &mut driver_handle => ("driver", r),
                    r = &mut resolver_handle => ("resolver", r),
                };
                match exit.1 {
                    Ok(()) => warn!(
                        subsystem = exit.0,
                        "follower subsystem exited cleanly (unexpected)"
                    ),
                    Err(e) => error!(subsystem = exit.0, error = ?e, "follower subsystem failed"),
                }
                marshal_handle.abort();
                executor_handle.abort();
                driver_handle.abort();
                resolver_handle.abort();
                shutdown.cancel();
            });

        Ok(CertFollowHandle { consensus_handle })
    }
}
