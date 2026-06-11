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
    application::{BeaconEngineLike, DerivedBlockBuilder, ExecutedChain},
    dpos::{
        derive_cold_start_heights, read_consensus_archive_last_finalized, wait_for_activation_block,
    },
    executor,
    order_block::{OrderBlock, K},
    outer::{follower_marshal_config, init_finalizations_archive, EpochSchemeProvider, MAX_REPAIR},
    scheme::epoch_committee_from_snapshot,
    OriginEpocher,
};
use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::{
    marshal::{core::Actor as MarshalActor, Update},
    types::{Epoch, Height},
    Reporters,
};
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use eyre::{eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::{fluent_namespace, scheme::build_verifier};
use fluentbase_staking_reader::{
    reader::{epoch_of_block, StakingReaderConfig},
    EpochTransition, RethStakingStateReader, ValidatorSetCache,
};
use reth_chain_state::CanonicalInMemoryState;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
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

/// Fatal only when reth's EL-sync makes NO forward progress for this long. An
/// absolute deadline is wrong for prod: a deep backfill (millions of blocks
/// behind the DPoS activation point) legitimately takes hours — the failure
/// signal is *stalled* progress, not elapsed time. Progress is read via
/// `last_block_number()` (NEVER `best_number`, which freezes during pipeline
/// backfill on reth 2.x — see project memory `reth-sync-progress`).
const EL_SYNC_NO_PROGRESS: Duration = Duration::from_secs(120);
/// Re-targeting rounds for the catch-up loop: each round chases a newer upstream
/// tip; the cap exit floors at the synced head and relies on the driver's
/// catch-up jump for the residual gap.
const CATCHUP_MAX_ROUNDS: u32 = 16;

/// Namespace type for the launch entry point.
pub struct CertFollowLayer;

impl CertFollowLayer {
    /// Launch the trustless cert-follower: read the reth cold-start anchor,
    /// register the anchor epoch's BLS verifier, stand up marshal + executor +
    /// resolver + driver + the epoch-rotation forwarder, and start them under an
    /// internal supervisor. `upstream` serves the resolver's by-height pulls;
    /// `finalized_rx` is the live finalized-cert stream the node's WS actor pushes.
    #[allow(clippy::too_many_arguments)]
    pub async fn launch<Provider, EvmConfig, BeaconEngine, D, XC, U>(
        ctx: Context,
        reth: CertFollowRethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: CertFollowConfig,
        deriver: D,
        executed: XC,
        upstream: U,
        finalized_rx: mpsc::Receiver<UpstreamFinalized>,
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
                ExecutionData = D::Derived,
            > + Clone
            + Send
            + Sync
            + 'static,
        D: DerivedBlockBuilder,
        XC: ExecutedChain,
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

        // Cold-start checkpoint: the block the follower trust-syncs reth to and floors
        // the marshal at. A fresh follower's reth is empty (post-merge reth won't sync
        // historical blocks without an Engine-API head), so a bounded catch-up loop
        // drives reth onto the upstream's LIVE tip, then floors at what reth actually
        // synced to. A single FCU+wait is NOT enough: reth syncs only to that one target
        // while the upstream keeps producing, leaving a stale `[synced+1 .. tip]` gap
        // that re-wedges if it exceeds MAX_REPAIR within one epoch. Each round backfills
        // a smaller span, so the residual gap converges to the steady-state sync latency.
        // A restart (reth already holds the archive anchor) skips the loop and resumes
        // there. Weak-subjectivity: ONLY the EL-sync head hash is trusted — reth validates
        // every block (state root) and the driver verifies every live cert from the
        // checkpoint forward (the first verified cert authenticates the checkpoint hash
        // via its `parent_hash`).
        let (checkpoint_height, checkpoint_hash) = if provider
            .block_hash(anchor_height)
            .wrap_err("block_hash(anchor) probe failed")?
            .is_none()
        {
            let mut synced_height = anchor_height;
            let mut rounds = 0u32;
            loop {
                let Some(latest) = upstream.get_latest().await else {
                    warn!(
                        "cert-follow: upstream getLatest returned none; relying on existing reth state"
                    );
                    break;
                };
                let latest_block = latest.block;
                // F-type: the upstream serves ORDERING artifacts — there is no
                // EVM body to new_payload and the digest is unresolvable by
                // reth. The only real EVM hash on the wire is the committee-
                // attested `result` (derived hash of tip − K); FCU toward it
                // and let reth devp2p backfill the bodies.
                let tip_hash = latest_block.result;
                let tip_height = latest_block.height.saturating_sub(K);
                if tip_hash == B256::ZERO {
                    info!(
                        tip = latest_block.height,
                        "cert-follow: upstream tip is inside the pre-K window; \
                         nothing to EL-sync"
                    );
                    break;
                }
                info!(
                    tip_height,
                    anchor_height,
                    round = rounds,
                    "cert-follow: driving reth EL-sync toward attested derived hash"
                );
                let _ = beacon_engine_handle
                    .fork_choice_updated(
                        ForkchoiceState {
                            head_block_hash: tip_hash,
                            safe_block_hash: tip_hash,
                            finalized_block_hash: genesis_hash,
                        })
                    .await;

                // Wait for reth's backward-sync to canonicalize the tip (block_hash
                // present ⇒ executed ⇒ committee state queryable). Fail only on a
                // *stall*: a deep prod backfill is legitimately slow, so the deadline
                // resets whenever `last_block_number()` advances.
                let mut last_progress = provider.last_block_number().unwrap_or(0);
                let mut stall_deadline = ctx.current() + EL_SYNC_NO_PROGRESS;
                while provider
                    .block_hash(tip_height)
                    .wrap_err("block_hash(tip) probe during EL-sync")?
                    .is_none()
                {
                    let now = provider.last_block_number().unwrap_or(last_progress);
                    if now > last_progress {
                        last_progress = now;
                        stall_deadline = ctx.current() + EL_SYNC_NO_PROGRESS;
                    } else if ctx.current() >= stall_deadline {
                        return Err(eyre!(
                            "cert-follow: reth EL-sync stalled for {EL_SYNC_NO_PROGRESS:?} at \
                             height {last_progress} (target tip {tip_height}); check devp2p \
                             peering (trusted peers / firewall) and upstream block-body availability"
                        ));
                    }
                    ctx.sleep(Duration::from_secs(2)).await;
                }
                synced_height = tip_height;
                rounds += 1;

                // Exit when the residual gap above the synced head is crossable: a
                // later-epoch upstream tip triggers the driver's catch-up `set_floor`-jump,
                // and a same-epoch gap ≤ MAX_REPAIR the marshal fills contiguously.
                // Otherwise loop — the upstream advanced > MAX_REPAIR in-epoch while we
                // synced, so converge onto the newer tip (a smaller backfill each round).
                let now_tip = upstream
                    .get_latest()
                    .await
                    .map(|u| u.block.height.saturating_sub(K))
                    .unwrap_or(tip_height);
                if catchup_gap_crossable(
                    tip_height,
                    now_tip,
                    interval,
                    dpos_activation_block,
                    MAX_REPAIR.get() as u64,
                ) {
                    break;
                }
                if rounds >= CATCHUP_MAX_ROUNDS {
                    warn!(
                        synced_height,
                        now_tip,
                        "cert-follow: catch-up loop hit round cap; flooring at synced head, \
                         relying on the catch-up jump for the residual gap"
                    );
                    break;
                }
            }
            let hash = wait_for_activation_block(&ctx, &provider, synced_height).await?;
            (synced_height, hash)
        } else {
            let hash = wait_for_activation_block(&ctx, &provider, anchor_height).await?;
            (anchor_height, hash)
        };

        // reth's actual canonical head (devp2p-synced; == the checkpoint after the
        // catch-up loop). Seed the executor head here so it never issues a backward FCU
        // to an ancestor (reth spec-skips that → wedge); finalized advances forward from
        // the checkpoint as certs verify.
        let chain = canonical_state.chain_info();
        let (head_num, head_hash) = (chain.best_number, chain.best_hash);
        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head at follower startup")?;

        let checkpoint_epoch = epoch_of_block(checkpoint_height, interval, dpos_activation_block);
        let initial_snapshot =
            reader.epoch_committee_snapshot(checkpoint_epoch, checkpoint_hash)?;
        if initial_snapshot.validators.is_empty() {
            return Err(eyre!(
                "cert-follow: checkpoint epoch {checkpoint_epoch} has no committed committee \
                 (read at checkpoint block {checkpoint_height}); point --cert-follow at a \
                 network whose committee is committed for that epoch"
            ));
        }
        info!(
            chain_id,
            checkpoint_height,
            head_num,
            checkpoint_epoch,
            interval,
            "cert-follower cold-start checkpoint resolved"
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
        // Pin the in-order floor to the checkpoint: dispatch from checkpoint+1 forward
        // (raises-only, so a restart with a higher archive floor is a no-op).
        marshal_mailbox
            .set_floor(Height::new(checkpoint_height))
            .await;

        // Register the checkpoint epoch's verifier BEFORE any cert is processed —
        // otherwise the marshal would reject the first finalization (no scheme).
        let namespace = fluent_namespace(chain_id);
        let initial_committee = epoch_committee_from_snapshot(&initial_snapshot)
            .map_err(|e| eyre!("checkpoint committee has non-unique participants: {e:?}"))?;
        scheme_provider.register(
            Epoch::new(checkpoint_epoch),
            build_verifier(&namespace, initial_committee.bimap),
        );

        // The checkpoint itself is not cert-verified here: its blocks `[..checkpoint]`
        // are trusted via the EL-sync head (reth state-root-validates each) and the
        // deferred L1 anchor source. Authentication is transitive forward: the driver
        // BLS-verifies every cert from `checkpoint+1`, and the consensus digest IS the
        // EVM block hash committing `parent_hash`, so the first verified descendant cert
        // authenticates the checkpoint's hash. The checkpoint *hash* source (the trusted
        // EL-sync head) is the remaining trust input — closed by the deferred L1 anchor
        // source, not by verifying the checkpoint's own cert.

        // Executor — drives reth from the verified finalized stream.
        let (executor_actor, executor_mailbox) = executor::Actor::init(
            ctx.with_label("executor"),
            executor::Config {
                beacon_engine: beacon_engine_handle,
                deriver,
                executed: executed.clone(),
                marshal: marshal_mailbox.clone(),
                fcu_heartbeat_interval,
                last_consensus_finalized_height,
                last_execution_finalized_height,
                initial_finalized: (Height::new(checkpoint_height), checkpoint_hash),
                initial_head: (Height::new(head_num), head_hash),
                fcu_pace,
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
        let provider_for_et = provider.clone();
        let mut epoch_transition = EpochTransition::new(
            reader_for_et,
            cache,
            stubs::NullPeerSetSink,
            fluentbase_p2p::constants::MAX_PEER_SET_SIZE as usize,
            Some(boundary_tx),
            Arc::new(move |n| provider_for_et.block_hash(n).ok().flatten()),
            K,
        );

        // Scheme-registration forwarder. Spawned BEFORE cold_start so the
        // cold_start boundary fire is drained (re-registering the checkpoint scheme is
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
            .cold_start(checkpoint_hash, checkpoint_height)
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
            Reporters::<Update<OrderBlock>, stubs::AppReporter, driver::MarshalReporter>::from((
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

/// Cold-start catch-up loop exit test: is the residual gap `(synced_tip, current_tip]`
/// crossable so the marshal can advance the floor from `synced_tip`?
///
/// True when EITHER the upstream's current tip is in a **later epoch** than the synced
/// head (the driver's catch-up `set_floor`-jump fires on the first later-epoch cert and
/// skips the gap) OR the same-epoch gap is `≤ max_repair` (the marshal fills it
/// contiguously in one repair window). The only non-crossable case — same epoch AND
/// gap `> max_repair` — is the wedge, so the loop keeps driving reth onto the
/// newer tip until this returns true.
fn catchup_gap_crossable(
    synced_tip: u64,
    current_tip: u64,
    interval: u32,
    dpos_activation_block: u64,
    max_repair: u64,
) -> bool {
    epoch_of_block(current_tip, interval, dpos_activation_block)
        > epoch_of_block(synced_tip, interval, dpos_activation_block)
        || current_tip.saturating_sub(synced_tip) <= max_repair
}

#[cfg(test)]
mod tests {
    use super::catchup_gap_crossable;

    // interval 32, activation 64 ⇒ epoch boundaries at 64,96,128,…; MAX_REPAIR 20.
    const INTERVAL: u32 = 32;
    const ACTIVATION: u64 = 64;
    const MAX_REPAIR: u64 = 20;

    fn crossable(synced: u64, current: u64) -> bool {
        catchup_gap_crossable(synced, current, INTERVAL, ACTIVATION, MAX_REPAIR)
    }

    #[test]
    fn same_epoch_small_gap_is_crossable() {
        // synced 70, current 85: both epoch 0, gap 15 ≤ MAX_REPAIR → marshal fills it.
        assert!(crossable(70, 85));
    }

    #[test]
    fn same_epoch_gap_over_max_repair_is_not_crossable() {
        // synced 70, current 95: both epoch 0 (95-64=31, /32=0), gap 25 > MAX_REPAIR →
        // the wedge condition; the loop must keep catching up.
        assert!(!crossable(70, 95));
    }

    #[test]
    fn later_epoch_is_crossable_even_when_gap_exceeds_max_repair() {
        // synced 70 (epoch 0), current 96 (epoch 1): the catch-up jump skips the gap,
        // so a 26-block gap is fine.
        assert!(crossable(70, 96));
    }

    #[test]
    fn no_advance_is_crossable() {
        // Upstream did not move while we synced (gap 0) — done.
        assert!(crossable(87, 87));
    }

    #[test]
    fn small_epoch_auto_crosses_a_boundary() {
        // interval 16 ≤ MAX_REPAIR: any gap > MAX_REPAIR necessarily crosses an epoch,
        // so the same-epoch-wedge case cannot arise.
        assert!(catchup_gap_crossable(64, 85, 16, ACTIVATION, MAX_REPAIR));
    }
}
