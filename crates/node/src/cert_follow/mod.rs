//! Host adapter for `--cert-follow`: spawns the dedicated OS thread +
//! commonware-tokio runtime, builds the reth handle + WS upstream, and calls
//! [`fluentbase_consensus::CertFollowLayer::launch`]. Parallels
//! [`crate::dpos::spawn_dpos`] but for a non-validator follower (no BLS/peer
//! keys, no p2p, no slasher — it verifies upstream certs and drives its own reth).

pub mod l1;
pub mod upstream;

use std::{path::PathBuf, time::Duration};

use commonware_runtime::{tokio::Context, Metrics as _, Spawner as _};
use eyre::WrapErr as _;
use fluentbase_consensus::{
    CertFollowConfig, CertFollowLayer, CertFollowRethHandle, UpstreamFinalized,
};
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode};
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::dpos::CanonicalStateAccess;

/// Operator-supplied follower configuration (parsed from CLI in `main.rs`).
pub struct CertFollowerConfig {
    /// `consensus` RPC state handle (serving side, D4). `Some` on every
    /// production follower: verified pairs feed a bounded window behind the
    /// same WS namespace validators serve. `None` = no serving (tests).
    pub feed: Option<crate::consensus_rpc::FeedStateHandle>,
    /// Upstream `consensus` RPC WebSocket URLs (validators or other
    /// followers). The WS actor rotates through them on connect failure /
    /// disconnect (failover).
    pub sequencer_urls: Vec<String>,
    /// Staking system-contract config (same JSON the validator uses) — the
    /// follower reads per-epoch committees from its own reth at this address.
    pub staking_config_path: PathBuf,
    /// L1 Rollup checkpoint source (D2). `None` = devnet fallback (the
    /// upstream `get_latest()` head stays the only trust input).
    pub l1: Option<l1::L1CheckpointConfig>,
    /// Path to the public threshold-beacon polynomial hex (`PK_epoch`, from
    /// `--dpos.beacon-sharing-path`). REQUIRED on a beacon-active chain so the
    /// follower verifies the seed half of each combined cert and reproduces
    /// `prev_randao = H(seed)`; `None` = pre-beacon / fallback chain. Parsed in
    /// `run_cert_follower_stack` (mirrors `staking_config_path`).
    pub beacon_sharing_path: Option<PathBuf>,
}

/// Parse the public threshold-beacon polynomial (`Sharing` / `PK_epoch`) from a
/// hex file. A follower never signs, so only the public polynomial is needed.
pub(crate) fn parse_beacon_sharing(
    path: &PathBuf,
) -> eyre::Result<
    commonware_cryptography::bls12381::primitives::sharing::Sharing<
        commonware_cryptography::bls12381::primitives::variant::MinSig,
    >,
> {
    let hex = std::fs::read_to_string(path).wrap_err("read beacon-sharing file")?;
    let bytes = commonware_utils::from_hex_formatted(hex.trim())
        .ok_or_else(|| eyre::eyre!("invalid beacon-sharing hex"))?;
    fluentbase_consensus::beacon::seed::parse_sharing(&bytes)
        .map_err(|e| eyre::eyre!("parse beacon Sharing: {e:?}"))
}

pub type CertFollowerSpawn<N, AddOns> = crate::utils::ConsensusSpawn<N, AddOns>;

/// Spawn the cert-follower thread. Like [`crate::dpos::spawn_dpos`], the thread
/// blocks on `handle_tx` until the reth `FullNode` is delivered, then stands up
/// the commonware runtime and runs [`run_cert_follower_stack`].
pub fn spawn_cert_follower<N, AddOns>(
    cfg: CertFollowerConfig,
    shutdown_token: CancellationToken,
) -> CertFollowerSpawn<N, AddOns>
where
    N: FullNodeComponents<
            Types: reth_node_api::NodeTypes<
                Payload = EthEngineTypes,
                Primitives = reth_ethereum_primitives::EthPrimitives,
            >,
        > + 'static,
    AddOns: RethRpcAddOns<N> + 'static,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm: reth_evm::ConfigureEvm<
            Primitives = EthPrimitives,
            NextBlockEnvCtx = reth_evm::NextBlockEnvAttributes,
        > + Clone
        + Send
        + Sync
        + 'static,
{
    crate::utils::spawn_consensus_thread("cert-follow", move |ctx, node| {
        run_cert_follower_stack(ctx, node, cfg, shutdown_token)
    })
}

/// The cert-follower thread body — runs on the commonware tokio runtime. Builds
/// the reth handle + WS upstream, launches the follower engine, then supervises
/// the engine + WS handles against the shutdown token.
async fn run_cert_follower_stack<N, AddOns>(
    ctx: Context,
    node: FullNode<N, AddOns>,
    cfg: CertFollowerConfig,
    shutdown_token: CancellationToken,
) -> eyre::Result<()>
where
    N: FullNodeComponents<
        Types: reth_node_api::NodeTypes<
            Payload = EthEngineTypes,
            Primitives = reth_ethereum_primitives::EthPrimitives,
        >,
    >,
    AddOns: RethRpcAddOns<N>,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm: reth_evm::ConfigureEvm<
            Primitives = EthPrimitives,
            NextBlockEnvCtx = reth_evm::NextBlockEnvAttributes,
        > + Clone
        + Send
        + Sync
        + 'static,
{
    let chain_id = node.chain_spec().chain_id();
    let staking_config = fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
        &cfg.staking_config_path,
    )
    .wrap_err_with(|| {
        format!(
            "failed loading staking config from {}",
            cfg.staking_config_path.display()
        )
    })?;

    let reth = CertFollowRethHandle {
        provider: node.provider.clone(),
        evm_config: node.evm_config.clone(),
        beacon_engine_handle: crate::importer::RethImporter::from_env(
            node.add_ons_handle.beacon_engine_handle.clone(),
        )?,
        chain_id,
        canonical_state: node.provider.canonical_state(),
        genesis_hash: node.chain_spec().genesis_hash(),
    };

    // WS upstream: subscribe + serve gap pulls. Spawned BEFORE launch so the
    // live stream is flowing as soon as the engine starts.
    let (ws_actor, upstream_handle, finalized_rx) =
        upstream::init(ctx.clone(), cfg.sequencer_urls.clone());
    let mut ws_handle = ws_actor.start();
    info!(urls = ?cfg.sequencer_urls, "cert-follow upstream started");

    let l1_checkpoint_hash = match &cfg.l1 {
        Some(l1_cfg) => Some(l1::fetch_checkpoint_hash(l1_cfg).await?),
        None => None,
    };
    // Threshold beacon: a follower never signs, so it holds only the PUBLIC
    // polynomial (`PK_epoch`). With it the per-epoch verifier accepts+verifies
    // the seed half of each combined cert and the deriver recovers
    // `prev_randao = H(seed)`; without it the follower would reject every
    // seeded cert and derive the `order.digest()` fallback (divergence).
    let deriver_base =
        crate::derive::RethBlockDeriver::new(node.provider.clone(), node.evm_config.clone());
    let (deriver, beacon) = match cfg.beacon_sharing_path.as_ref().map(parse_beacon_sharing) {
        Some(sharing) => {
            let sharing = sharing?;
            let seed_namespace = fluentbase_consensus::beacon::seed::seed_namespace(
                &fluentbase_bls::fluent_namespace(chain_id),
            );
            // Per-epoch resolver: read PK_E from L2 state for the block's epoch,
            // genesis-PK_0 fallback when uncommitted (every epoch pre-rotation).
            let genesis_pk = *sharing.public();
            let beacon_reader = fluentbase_staking_reader::reader::RethStakingStateReader::new(
                node.provider.clone(),
                node.evm_config.clone(),
                staking_config.clone(),
            );
            let resolver = std::sync::Arc::new(
                move |epoch: u64, at: alloy_primitives::B256| {
                    beacon_reader
                        .epoch_beacon_key(epoch, at)
                        .ok()
                        .flatten()
                        .or(Some(genesis_pk))
                },
            );
            let deriver = deriver_base.with_beacon_resolver(seed_namespace.clone(), resolver);
            (deriver, Some((sharing, None, seed_namespace)))
        }
        None => (deriver_base, None),
    };

    let follow_cfg = CertFollowConfig {
        staking_config,
        l1_checkpoint_hash,
        fcu_heartbeat_interval: Duration::from_secs(8),
        stop_at_next_boundary: false,
        beacon,
    };

    let executed = crate::ordering::ProviderExecutedChain(node.provider.clone());

    // Serving side (D4): verified pairs → bounded window + the same
    // `consensus` namespace events validators emit. `record_finalized`
    // already emits both finality tiers, so a downstream follower gets
    // `ResultFinalized` for free.
    let verified_tx = cfg.feed.map(|handle| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<UpstreamFinalized>();
        let window: crate::consensus_rpc::state::CertWindow = Default::default();
        handle.set_window(window.clone());
        drop(ctx.with_label("window_feed").spawn(move |_| async move {
            while let Some(uf) = rx.recv().await {
                let height = uf.block.height;
                let cb = std::sync::Arc::new(crate::certified_block::CertifiedBlock::from_parts(
                    &uf.finalization,
                    &uf.block,
                ));
                {
                    let mut w = window.write().expect("cert window poisoned");
                    w.insert(height, cb.clone());
                    while w.len() as u64 > fluentbase_consensus::cert_follow::JUMP_THRESHOLD {
                        w.pop_first();
                    }
                }
                handle.record_finalized(cb, crate::consensus_rpc::now_ms());
            }
        }));
        tx
    });

    let mut handle = CertFollowLayer::launch(
        ctx,
        reth,
        follow_cfg,
        deriver,
        executed,
        upstream_handle,
        finalized_rx,
        verified_tx,
        shutdown_token.clone(),
    )
    .await?;

    let exit_reason = tokio::select! {
        _ = shutdown_token.cancelled() => {
            info!("cert-follow thread received shutdown signal, exiting");
            "shutdown_token"
        }
        res = &mut handle.consensus_handle => {
            match res {
                Ok(()) => warn!("cert-follow engine exited cleanly (unexpected)"),
                Err(e) => error!(error = ?e, "cert-follow engine task failed"),
            }
            shutdown_token.cancel();
            "engine_exit"
        }
        res = &mut ws_handle => {
            match res {
                Ok(()) => warn!("cert-follow WS upstream exited cleanly (unexpected)"),
                Err(e) => error!(error = ?e, "cert-follow WS upstream task failed"),
            }
            shutdown_token.cancel();
            "ws_exit"
        }
    };

    handle.consensus_handle.abort();
    ws_handle.abort();
    info!(reason = exit_reason, "cert-follow thread exiting");
    Ok(())
}
