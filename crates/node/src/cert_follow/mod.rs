//! Follower overlay for `--cert-follow` (one of the two overlay shapes
//! [`crate::dpos::run_node_stack`] selects). Builds the reth handle + WS
//! upstream + ONE gossip-idle broadcast `Muxer`, and calls
//! [`fluentbase_consensus::dpos::DposLayer::launch_follower`] (inlet+executor).
//! A non-validator follower has no BLS/signer key, no beacon plane, no slasher
//! — the cert-inlet BLS-verifies upstream certs and drives the executor, the
//! sole reth writer. Unlike the former standalone thread body, this returns its
//! handles to the unified node-stack supervisor instead of running its own.

pub mod l1;
pub mod upstream;

use crate::dpos::{CanonicalStateAccess, CertInletCfg, FollowerCfg, SupervisedHandle};
use commonware_cryptography::Signer as _;
use commonware_p2p::{
    utils::mux::Muxer,
    Ingress,
};
use commonware_runtime::{tokio::Context, Metrics as _, Spawner as _};
use eyre::WrapErr as _;
use fluentbase_consensus::{
    dpos::{DposLayer, DposLayerHandle, FollowerLayerConfig, FollowerRethHandle},
    UpstreamFinalized,
};
use fluentbase_p2p::{FluentP2P, FluentP2PConfig};
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode};
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Build the FOLLOWER overlay: WS upstream + L1 checkpoint + the ONE gossip-idle
/// broadcast `Muxer`, then launch the inlet-only engine via
/// [`DposLayer::launch_follower`]. Returns the engine handle + the WS / network /
/// broadcast-mux handles for the shared node-stack supervisor (it does NOT run
/// its own `select!` — that is the unified `supervise` in `dpos.rs`).
pub(crate) async fn launch_follower_overlay<N, AddOns>(
    ctx: Context,
    node: FullNode<N, AddOns>,
    cfg: FollowerCfg,
    inlet: CertInletCfg,
    shutdown_token: CancellationToken,
) -> eyre::Result<(DposLayerHandle, Vec<SupervisedHandle>)>
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

    let beacon_engine_handle = crate::importer::RethImporter::from_env(
        node.add_ons_handle.beacon_engine_handle.clone(),
    )?;
    let reth = FollowerRethHandle {
        provider: node.provider.clone(),
        evm_config: node.evm_config.clone(),
        beacon_engine_handle,
        chain_id,
        canonical_state: node.provider.canonical_state(),
        genesis_hash: node.chain_spec().genesis_hash(),
    };

    // WS upstream: ONE `upstream::init`, shared by the cold-start JUMP
    // (`upstream_handle.get_latest`) and the cert-inlet (`finalized_rx`, the
    // inlet's SOLE producer). Spawned BEFORE launch so the live stream flows as
    // soon as the engine starts.
    let (ws_actor, upstream_handle, finalized_rx, conn_gen) =
        upstream::init(ctx.clone(), inlet.urls.clone());
    let ws_handle = ws_actor.start();
    info!(urls = ?inlet.urls, "cert-follow upstream started");

    let l1_checkpoint_hash = match &cfg.l1 {
        Some(l1_cfg) => Some(l1::fetch_checkpoint_hash(l1_cfg).await?),
        None => None,
    };
    // The deriver computes prev_randao = H(seed) directly from the
    // cert-recovered seed (no on-chain PK_E read — that layer is gone,
    // DPOS_ARCHITECTURE §8.11), matching the committee.
    let deriver =
        crate::derive::RethBlockDeriver::new(node.provider.clone(), node.evm_config.clone());
    let executed = crate::ordering::ProviderExecutedChain(node.provider.clone());
    let assembler = std::sync::Arc::new(crate::ordering::PoolAssembler::new(
        node.pool.clone(),
        executed.clone(),
    ));
    let fee_recipient = fluentbase_types::PRECOMPILE_FEE_MANAGER;
    let target_gas_limit = node.chain_spec().genesis().gas_limit;

    // B3 — serving side (D4): the cert-inlet feeds each VERIFIED pair here →
    // bounded window + the same `consensus` namespace events validators emit. A
    // TIER-2 follower aligns by reading THIS node's window WS. `record_finalized`
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
                    while w.len() as u64 > fluentbase_consensus::JUMP_THRESHOLD {
                        w.pop_first();
                    }
                }
                handle.record_finalized(cb, crate::consensus_rpc::now_ms());
            }
        }));
        tx
    });

    // The ONE plane piece a follower keeps: a minimal, gossip-idle broadcast
    // `Muxer`. Required so the `buffered::Engine` is ALIVE to answer the marshal's
    // buffer-first body lookup with `None` (then either the verified-cache the
    // inlet populated via `verified()` resolves the body locally, or — for the
    // floor→frontier gap — the marshal's `UpstreamResolver` backfills it
    // by-height from the cert upstream). A standalone `--cert-follow` follower carries NO operator peer
    // key, so we mint an EPHEMERAL ed25519 identity (it never gossips: no
    // consensus bootstrappers, so the gossip arm never fires). NO
    // vote/cert/resolver/marshal muxes, NO DkgActor, NO beacon oracle, NO signer.
    let peer_keypair = fluentbase_p2p::generate_ephemeral_ed25519_key();
    let me = peer_keypair.public_key();
    let listen = std::net::SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        0, // ephemeral port — the follower never accepts consensus dials
    );
    let (p2p, handles) = FluentP2P::build(
        ctx.clone(),
        FluentP2PConfig {
            crypto: peer_keypair,
            chain_id,
            listen,
            dialable: Ingress::Socket(listen),
            bootstrappers: vec![],
        },
    );
    let net_handle = p2p.start();
    let oracle = handles.oracle.clone();
    let (mux_bcast, broadcast_mux) = Muxer::new(
        ctx.with_label("follower_broadcast_mux"),
        handles.broadcast_sender,
        handles.broadcast_receiver,
        256usize,
    );
    let bcast_mux_handle = ctx.with_label("follower_broadcast_mux_sup").spawn(move |_| async move {
        if let Ok(Err(e)) = mux_bcast.start().await {
            warn!(error = ?e, "follower broadcast Muxer p2p receiver failed");
        }
    });
    let broadcast_mux = std::sync::Arc::new(tokio::sync::Mutex::new(broadcast_mux));

    let follow_cfg = FollowerLayerConfig {
        me,
        staking_config,
        l1_checkpoint_hash,
        deriver,
        executed,
        assembler,
        fee_recipient,
        target_gas_limit,
        feed: None,
        fcu_heartbeat_interval: Duration::from_secs(8),
        upstream: Some(upstream_handle),
        finalized_rx,
        conn_gen: Some(conn_gen),
        verified_tx,
    };

    // A `--cert-follow` follower ALWAYS BLS-verifies upstream certs (the inlet's
    // SOLE producer); there is no no-verify mode in v1 — the standalone
    // `--sequencer-url` trust relay is the separate `launch_consensus_node` path,
    // not this overlay.

    let handle = DposLayer::launch_follower(
        ctx,
        reth,
        follow_cfg,
        oracle,
        broadcast_mux,
        shutdown_token,
    )
    .await?;

    // Hand the WS / network / broadcast-mux handles back to the unified
    // node-stack supervisor (`dpos.rs::supervise`); the engine handle rides in
    // the returned `DposLayerHandle`.
    let supervised: Vec<SupervisedHandle> = vec![
        ("ws_upstream", ws_handle),
        ("network", net_handle),
        ("mux", bcast_mux_handle),
    ];
    Ok((handle, supervised))
}
