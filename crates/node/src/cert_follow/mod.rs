//! Host adapter for `--cert-follow`: spawns the dedicated OS thread +
//! commonware-tokio runtime, builds the reth handle + WS upstream, and calls
//! [`fluentbase_consensus::CertFollowLayer::launch`]. Parallels
//! [`crate::dpos::spawn_dpos`] but for a non-validator follower (no BLS/peer
//! keys, no p2p, no slasher — it verifies upstream certs and drives its own reth).

pub mod upstream;

use std::{path::PathBuf, thread, time::Duration};

use commonware_runtime::{
    tokio::{Config as RuntimeConfig, Context, Runner as TokioRunner},
    Runner as _,
};
use eyre::WrapErr as _;
use fluentbase_consensus::{CertFollowConfig, CertFollowLayer, CertFollowRethHandle};
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode};
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::dpos::CanonicalStateAccess;

/// Marshal storage prefix — identical to the validator
/// (`fluentbase_consensus::dpos` `MARSHAL_PARTITION_PREFIX`) so a node can switch
/// between validator and follower modes on the same data dir without migration.
const MARSHAL_PARTITION_PREFIX: &str = "consensus_marshal";

/// Operator-supplied follower configuration (parsed from CLI in `main.rs`).
pub struct CertFollowerConfig {
    /// Upstream `consensus` RPC WebSocket URL (a validator or another follower).
    pub sequencer_url: String,
    /// Staking system-contract config (same JSON the validator uses) — the
    /// follower reads per-epoch committees from its own reth at this address.
    pub staking_config_path: PathBuf,
}

pub struct CertFollowerSpawn<N, AddOns>
where
    N: FullNodeComponents,
    AddOns: RethRpcAddOns<N>,
{
    pub join: thread::JoinHandle<eyre::Result<()>>,
    pub handle_tx: oneshot::Sender<FullNode<N, AddOns>>,
    pub dead_rx: oneshot::Receiver<()>,
}

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
    let (handle_tx, handle_rx) = oneshot::channel::<FullNode<N, AddOns>>();
    let (dead_tx, dead_rx) = oneshot::channel::<()>();
    let shutdown_token_inner = shutdown_token.clone();

    let join = thread::Builder::new()
        .name("cert-follow".into())
        .spawn(move || {
            let node = handle_rx
                .blocking_recv()
                .wrap_err("channel closed before reth FullNode could be received")?;

            let consensus_storage = node.data_dir.data_dir().join("dpos");
            info!(path = %consensus_storage.display(), "cert-follow storage directory determined");

            let runtime_config = RuntimeConfig::default()
                .with_tcp_nodelay(Some(true))
                .with_storage_directory(consensus_storage)
                .with_catch_panics(true);
            let runner = TokioRunner::new(runtime_config);
            let ret = runner.start(|ctx| async move {
                run_cert_follower_stack(ctx, node, cfg, shutdown_token_inner).await
            });
            let _ = dead_tx.send(());
            ret
        })
        .expect("failed to spawn cert-follow thread");

    CertFollowerSpawn {
        join,
        handle_tx,
        dead_rx,
    }
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
        upstream::init(ctx.clone(), cfg.sequencer_url.clone());
    let mut ws_handle = ws_actor.start();
    info!(url = %cfg.sequencer_url, "cert-follow upstream started");

    let follow_cfg = CertFollowConfig {
        staking_config,
        partition_prefix: MARSHAL_PARTITION_PREFIX.into(),
        mailbox_size: 256,
        fcu_heartbeat_interval: Duration::from_secs(8),
        fcu_pace: Duration::from_millis(20),
    };

    let deriver =
        crate::derive::RethBlockDeriver::new(node.provider.clone(), node.evm_config.clone());
    let executed = crate::ordering::ProviderExecutedChain(node.provider.clone());

    let mut handle = CertFollowLayer::launch(
        ctx,
        reth,
        follow_cfg,
        deriver,
        executed,
        upstream_handle,
        finalized_rx,
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
