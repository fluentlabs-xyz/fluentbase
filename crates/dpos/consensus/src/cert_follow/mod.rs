//! Trustless cert-follower engine (`--cert-follow`).
//!
//! A non-validator node that **verifies, not trusts**: a single sequential
//! reconciler loop ([`follow::FollowLoop`]) pulls `(cert, OrderBlock)` pairs
//! from an upstream `consensus` RPC, BLS-verifies every certificate against
//! the on-chain epoch committee, cross-checks the committee-attested `result`
//! hash against its OWN derivation, and drives reth via two-tier FCU. The
//! cold-start anchor is authenticated transitively (the first verified
//! descendant cert commits the chain via `result`); its *hash source* is the
//! remaining trust input — closed by the optional L1 Rollup checkpoint assert,
//! with the upstream `get_latest()` as the devnet fallback.
//!
//! The WS transport lives node-side behind the [`CertUpstream`] trait; this
//! module is transport-agnostic.

mod follow;
mod upstream;

use crate::{
    application::{BeaconEngineLike, DerivedBlockBuilder, ExecutedChain},
    dpos::{derive_cold_start_heights, wait_for_activation_block},
    order_block::K,
    scheme::epoch_committee_from_snapshot,
};
use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use eyre::{ensure, eyre, WrapErr as _};
use fluentbase_bls::{
    fluent_namespace,
    scheme::build_verifier,
    Scheme as BlsScheme,
};
use fluentbase_staking_reader::{
    reader::{epoch_of_block, StakingReaderConfig},
    RethStakingStateReader,
};
pub(crate) use follow::{CommitteeSource, ElSync};
pub use follow::{FollowExit, VerifiedTx, JUMP_THRESHOLD};
use reth_chain_state::CanonicalInMemoryState;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
use reth_storage_api::{
    BlockHashReader, BlockNumReader, BlockReader, HeaderProvider, StateProviderFactory,
};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::mpsc;
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
    /// L1 Rollup checkpoint hash (D2), already resolved node-side
    /// (`Rollup.getBatch(lastFinalizedBatchIndex()).toBlockHash`). `None` =
    /// devnet fallback: the upstream `get_latest()` head stays the only trust
    /// input, as today. The assert runs post-EL-sync: the block must exist
    /// canonically in the follower's own reth.
    pub l1_checkpoint_hash: Option<B256>,
    pub fcu_heartbeat_interval: Duration,
    /// Unified-supervisor lap mode: stop the loop at the next epoch
    /// boundary − 1 (relative to the launch checkpoint) and report the stop
    /// point via [`CertFollowHandle::stopped_rx`] instead of cancelling the
    /// shutdown token. `false` = standalone `--cert-follow` (run forever).
    pub stop_at_next_boundary: bool,
    /// Public threshold-beacon key (`PK_epoch` polynomial + seed namespace, no
    /// `Share` — a follower never signs). REQUIRED on a beacon-active chain:
    /// the per-epoch verifier scheme verifies the seed half of each combined
    /// finalization cert against it, and the deriver recovers
    /// `prev_randao = H(seed)` from it. `None` ⇒ the follower would reject
    /// every seeded cert and derive the fallback (divergence).
    pub beacon: Option<fluentbase_bls::scheme::BeaconKey>,
}

/// Handle the host adapter supervises alongside its WS-upstream actor.
pub struct CertFollowHandle {
    pub consensus_handle: Handle<()>,
    /// Fires once with the stop point when `stop_at_next_boundary` was set
    /// and the loop reached it (sender dropped without firing on error).
    pub stopped_rx: tokio::sync::oneshot::Receiver<follow::FollowExit>,
}

/// Fatal only when reth's EL-sync makes NO forward progress for this long. An
/// absolute deadline is wrong for prod: a deep backfill (millions of blocks
/// behind the DPoS activation point) legitimately takes hours — the failure
/// signal is *stalled* progress, not elapsed time. Progress is read via
/// `last_block_number()` (NEVER `best_number`, which freezes during pipeline
/// backfill on reth 2.x — see project memory `reth-sync-progress`).
const EL_SYNC_NO_PROGRESS: Duration = Duration::from_secs(120);

/// [`CommitteeSource`] over the follower's own state: committee snapshot at
/// the given executed hash → BLS verifier.
struct RethCommitteeSource<Provider, EvmConfig> {
    reader: RethStakingStateReader<Provider, EvmConfig>,
    namespace: Vec<u8>,
}

impl<Provider, EvmConfig> CommitteeSource for RethCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    fn scheme_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme> {
        let snap = self.reader.epoch_committee_snapshot(epoch, at_hash)?;
        ensure!(
            !snap.validators.is_empty(),
            "epoch {epoch} has no committed committee at {at_hash}"
        );
        let committee = epoch_committee_from_snapshot(&snap)
            .map_err(|e| eyre!("epoch {epoch} committee has non-unique participants: {e:?}"))?;
        // MULTISIG-ONLY verifier: `verify_certificate` ignores the seed now that
        // the PK_E layer is gone, so the cert-follower needs no per-epoch beacon
        // key — no in-block PK_E, no on-chain getEpochBeaconKey read.
        Ok(build_verifier(&self.namespace, committee.bimap, None))
    }
}

/// [`ElSync`] over the follower's reth: FCU toward the attested derived hash
/// (the upstream tip's `result` = derived hash of tip − K) and wait for
/// devp2p backfill to canonicalize it, failing only on stalled progress.
struct RethElSync<Provider, BeaconEngine> {
    ctx: Context,
    provider: Provider,
    beacon_engine: BeaconEngine,
    genesis_hash: B256,
    activation: u64,
}

impl<Provider, BeaconEngine> RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
{
    /// The height/hash reth currently sits at, clamped to ≥ activation (the
    /// ordering chain starts there; pre-activation blocks carry no certs).
    fn local_landing(&self) -> eyre::Result<(u64, B256)> {
        let tip = self
            .provider
            .last_block_number()
            .wrap_err("provider failed to report chain head")?
            .max(self.activation);
        let hash = self
            .provider
            .block_hash(tip)?
            .ok_or_else(|| eyre!("reth does not hold its own reported tip {tip}"))?;
        Ok((tip, hash))
    }
}

impl<Provider, BeaconEngine> ElSync for RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
{
    async fn sync_to(&self, latest: &UpstreamFinalized) -> eyre::Result<(u64, B256)> {
        // F-type: the upstream serves ORDERING artifacts — the only real EVM
        // hash on the wire is the committee-attested `result` (derived hash
        // of tip − K); FCU toward it and let reth devp2p backfill the bodies.
        let tip_hash = latest.block.result;
        let tip_height = latest.block.height.saturating_sub(K);
        if tip_hash == B256::ZERO {
            info!(
                tip = latest.block.height,
                "cert-follow: upstream tip is inside the pre-K window; nothing to EL-sync"
            );
            return self.local_landing();
        }
        if self
            .provider
            .block_hash(tip_height)
            .wrap_err("block_hash(tip) probe before EL-sync")?
            .is_some()
        {
            return self.local_landing();
        }
        info!(
            tip_height,
            "cert-follow: driving reth EL-sync toward attested derived hash"
        );
        let _ = self
            .beacon_engine
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: tip_hash,
                safe_block_hash: tip_hash,
                finalized_block_hash: self.genesis_hash,
            })
            .await;

        // Wait for reth's backward-sync to canonicalize the tip (block_hash
        // present ⇒ executed ⇒ committee state queryable). Fail only on a
        // *stall*: the deadline resets whenever `last_block_number()` advances.
        let mut last_progress = self.provider.last_block_number().unwrap_or(0);
        let mut stall_deadline = self.ctx.current() + EL_SYNC_NO_PROGRESS;
        while self
            .provider
            .block_hash(tip_height)
            .wrap_err("block_hash(tip) probe during EL-sync")?
            .is_none()
        {
            let now = self.provider.last_block_number().unwrap_or(last_progress);
            if now > last_progress {
                last_progress = now;
                stall_deadline = self.ctx.current() + EL_SYNC_NO_PROGRESS;
            } else if self.ctx.current() >= stall_deadline {
                return Err(eyre!(
                    "cert-follow: reth EL-sync stalled for {EL_SYNC_NO_PROGRESS:?} at \
                     height {last_progress} (target tip {tip_height}); check devp2p \
                     peering (trusted peers / firewall) and upstream block-body availability"
                ));
            }
            self.ctx.sleep(Duration::from_secs(2)).await;
        }
        // Clamp BEFORE resolving the hash so the returned pair is always
        // self-consistent (height and hash of the SAME block) — callers seed
        // the cursor from it directly.
        let landing = tip_height.max(self.activation);
        let hash = self
            .provider
            .block_hash(landing)?
            .ok_or_else(|| eyre!("EL-sync landed but block {landing} vanished"))?;
        Ok((landing, hash))
    }

    fn holds(&self, hash: B256) -> eyre::Result<bool> {
        Ok(self
            .provider
            .block_number(hash)
            .wrap_err("block_number(l1 checkpoint) probe failed")?
            .is_some())
    }
}

/// Codeless-tolerant epoch-geometry read: `None` when `ChainConfig` is not
/// deployed (or DPoS not yet scheduled) at `at` — the launch discriminator
/// between "restart datadir / genesis-baked devnet" and "fresh datadir on a
/// runtime-deployed chain", where geometry is only readable AFTER EL-sync.
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

/// Supervisor lap stop: the cursor height at which the FINALIZED tier
/// (`cursor − K`, two-tier FCU) sits EXACTLY on the last block of the
/// checkpoint's epoch (`next boundary − 1`). That block is the only valid
/// promotion anchor: the synthesized anchor OrderBlock at it is the
/// "starting epoch block" the per-epoch Inline requires, and the
/// EpochTransition boundary-resume rule bootstraps the NEXT epoch from it —
/// exactly the epoch the new member belongs to. Always strictly past the
/// checkpoint (`epoch_of(checkpoint)` puts the next boundary above it, and
/// `K ≥ 1`), so re-laps cannot spin.
fn next_lap_stop(checkpoint: u64, activation: u64, interval: u32) -> u64 {
    activation + (epoch_of_block(checkpoint, interval, activation) + 1) * interval as u64 - 1 + K
}

/// Namespace type for the launch entry point.
pub struct CertFollowLayer;

impl CertFollowLayer {
    /// Launch the lean cert-follower: read epoch geometry, EL-sync reth onto
    /// the upstream's attested tip, assert the L1 checkpoint (when
    /// configured), seed the cursor + initial epoch verifier, and spawn the
    /// reconciler loop. `upstream` serves by-height pulls; `finalized_rx` is
    /// the live finalized-cert stream the node's WS actor pushes;
    /// `verified_tx` (optional) receives every verified pair for the node's
    /// serving window.
    #[allow(clippy::too_many_arguments)]
    pub async fn launch<Provider, EvmConfig, BeaconEngine, D, XC, U>(
        ctx: Context,
        reth: CertFollowRethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: CertFollowConfig,
        deriver: D,
        executed: XC,
        upstream: U,
        finalized_rx: mpsc::Receiver<UpstreamFinalized>,
        verified_tx: Option<VerifiedTx>,
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
        BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
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

        // Epoch geometry. On a RESTART datadir `ChainConfig` is readable from
        // local state; on a FRESH datadir it is readable at genesis only when
        // baked there (devnet). Production deploys the cluster at runtime, so
        // a fresh follower must EL-sync FIRST and read geometry at the synced
        // landing — `read_geometry` is codeless-tolerant to discriminate.
        let (_rf_num, rf_hash, _h0_num, _h0_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);
        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            cfg.staking_config.clone(),
        );
        let mk_el_sync = |activation: u64| RethElSync {
            ctx: ctx.clone(),
            provider: provider.clone(),
            beacon_engine: beacon_engine_handle.clone(),
            genesis_hash,
            activation,
        };

        // Cold-start checkpoint: EL-sync onto the upstream's attested tip
        // (single shot — a residual gap is the loop's job: pulls when below
        // JUMP_THRESHOLD, another jump otherwise). Weak-subjectivity: ONLY the
        // EL-sync head hash is trusted — reth state-root-validates every body,
        // the loop verifies every cert from the checkpoint forward, and the
        // L1 assert (below) pins the landing when configured.
        let (activation, interval, checkpoint_height, checkpoint_hash) =
            match read_geometry(&reader, rf_hash)? {
                Some((activation, interval)) => {
                    let el_sync = mk_el_sync(activation);
                    let (h, hash) = match upstream.get_latest().await {
                        Some(latest) => el_sync.sync_to(&latest).await?,
                        None => {
                            warn!(
                                "cert-follow: upstream getLatest returned none; relying on \
                                 existing reth state"
                            );
                            let hash =
                                wait_for_activation_block(&ctx, &provider, activation).await?;
                            (activation, hash)
                        }
                    };
                    (activation, interval, h, hash)
                }
                None => {
                    // Fresh datadir on a runtime-deployed chain: sync without
                    // an activation clamp (unknown yet), then read geometry
                    // at the landing and re-clamp.
                    let latest = upstream.get_latest().await.ok_or_else(|| {
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
        let el_sync = mk_el_sync(activation);

        // L1 Rollup checkpoint assert (D2): the L1-finalized batch's last
        // block hash must be canonical in OUR reth. Height is resolved
        // locally — BatchRecord carries no absolute L2 height.
        if let Some(l1_hash) = cfg.l1_checkpoint_hash {
            let num = provider
                .block_number(l1_hash)
                .wrap_err("block_number(l1 checkpoint) probe failed")?;
            match num {
                Some(n) => info!(
                    hash = ?l1_hash,
                    height = n,
                    "cert-follow: L1 Rollup checkpoint verified against local chain"
                ),
                None => {
                    return Err(eyre!(
                        "cert-follow: L1 Rollup checkpoint {l1_hash:?} is NOT in the local \
                         chain after EL-sync — the synced head does not extend the \
                         L1-finalized history (possible upstream equivocation); refusing to follow"
                    ))
                }
            }
        }

        let checkpoint_epoch = epoch_of_block(checkpoint_height, interval, activation);
        let committees = RethCommitteeSource {
            reader,
            namespace: fluent_namespace(chain_id),
        };
        let initial_scheme = committees
            .scheme_at(checkpoint_epoch, checkpoint_hash)
            .wrap_err_with(|| {
                format!(
                    "checkpoint epoch {checkpoint_epoch} verifier (read at block \
                     {checkpoint_height}); point --cert-follow at a network whose committee \
                     is committed for that epoch"
                )
            })?;
        info!(
            chain_id,
            checkpoint_height, checkpoint_epoch, interval, "cert-follower cold-start resolved"
        );

        // Two-tier seed: the landing block's own result attestation arrives
        // only K blocks later, so the finalized tier starts at landing − K
        // (clamped to activation) — claiming the landing itself would
        // overstate finality by K on every restart.
        let finalized_floor = checkpoint_height.saturating_sub(K).max(activation);
        let finalized_hash = provider
            .block_hash(finalized_floor)?
            .ok_or_else(|| eyre!("reth does not hold the finality floor {finalized_floor}"))?;
        let seed_fcu = ForkchoiceState {
            head_block_hash: checkpoint_hash,
            safe_block_hash: finalized_hash,
            finalized_block_hash: finalized_hash,
        };
        let follow_loop = follow::FollowLoop {
            ctx: ctx.clone(),
            beacon_engine: beacon_engine_handle,
            deriver,
            executed,
            upstream,
            committees,
            el_sync,
            finalized_rx,
            verified_tx,
            schemes: BTreeMap::from([(checkpoint_epoch, initial_scheme)]),
            geometry: follow::Geometry {
                interval,
                activation,
                finalized_floor,
            },
            cursor: follow::Cursor {
                height: checkpoint_height,
                evm_hash: checkpoint_hash,
                prev_digest: None,
                finalized_hash,
            },
            last_fcu: seed_fcu,
            fcu_heartbeat_interval: cfg.fcu_heartbeat_interval,
            highest_live_seen: checkpoint_height,
            l1_checkpoint: cfg.l1_checkpoint_hash,
            starvation_jump: follow::STARVATION_JUMP,
            stop_at: cfg
                .stop_at_next_boundary
                .then(|| next_lap_stop(checkpoint_height, activation, interval)),
        };

        // The loop handle IS the engine handle: fail-closed — on any loop
        // error cancel the shared shutdown so the host brings the node down.
        // A boundary stop (supervisor lap mode) is the one NON-fatal exit: it
        // reports the stop point and leaves the token alone.
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        let consensus_handle = ctx.with_label("follow_loop").spawn(move |_| async move {
            match follow_loop.run().await {
                Ok(exit @ follow::FollowExit::StoppedAt { .. }) => {
                    let _ = stopped_tx.send(exit);
                    return;
                }
                Err(e) => error!(error = ?e, "cert-follower loop failed (fail-closed)"),
            }
            shutdown.cancel();
        });

        Ok(CertFollowHandle {
            consensus_handle,
            stopped_rx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::next_lap_stop;

    #[test]
    fn lap_stop_puts_the_finalized_anchor_on_boundary_minus_one() {
        // act=64, interval=32, K=3: checkpoint mid-epoch-0 (70) → next
        // boundary 96 → stop at 98 (finalized anchor = 98−K = 95 = the last
        // block of epoch 0, the only valid promotion anchor).
        assert_eq!(next_lap_stop(70, 64, 32), 98);
        // Checkpoint at boundary−1 (95) is still epoch 0 → same stop.
        assert_eq!(next_lap_stop(95, 64, 32), 98);
        // Checkpoint ON the boundary (96 = epoch 1) → anchor 127 → stop 130:
        // strictly past any epoch-1 checkpoint (no re-lap spin).
        assert_eq!(next_lap_stop(96, 64, 32), 130);
    }
}
