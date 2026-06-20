//! DPoS host adapter: spawns the dedicated OS thread + commonware-tokio
//! runtime, loads operator keys and configs from disk, builds the
//! `RethHandle` + `DposLayerConfig`, calls
//! [`fluentbase_consensus::dpos::DposLayer::launch`], then runs the
//! shutdown supervisor `select!`.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use commonware_runtime::tokio::Context;
// `Clock` (ctx.current/sleep in the finalized-height poller) + `Metrics`
// (ctx.with_label, ctx.encode) + `Spawner` (ctx.spawn) — used by the always-on
// beacon plane, the cert-feed actor, and the feature-gated devnet metrics endpoint.
use commonware_consensus::types::Height;
use commonware_p2p::{
    utils::mux::{Builder, Muxer},
    Ingress,
};
use commonware_runtime::{Clock as _, Handle, Metrics as _, Spawner as _};
use eyre::{eyre, OptionExt as _, WrapErr as _};
use fluentbase_consensus::dpos::{
    DposLayer, DposLayerConfig, DposLayerHandle, RethHandle, ResettableForward, SharedBeaconPlane,
    VoteBackupItem,
};
pub use fluentbase_consensus::FeedSink;
use fluentbase_p2p::{FluentP2P, FluentP2PConfig};
use fluentbase_staking_reader::{
    reader::RethStakingStateReader, EpochTransition, ValidatorSetCache,
};

use crate::consensus_rpc::{feed_actor::FeedActor, FeedStateHandle};
use reth_chain_state::CanonicalInMemoryState;
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode};
use reth_provider::providers::{BlockchainProvider, ProviderNodeTypes};
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Bridge trait exposing reth's `canonical_in_memory_state` snapshot to
/// the generic host adapter. No reth trait exposes this method —
/// `canonical_in_memory_state` is an inherent method on the concrete
/// `BlockchainProvider<N>`. Implemented below for that type so the
/// host adapter's `where` clause can require it.
pub trait CanonicalStateAccess: Send + Sync {
    fn canonical_state(&self) -> CanonicalInMemoryState<EthPrimitives>;
}

impl<N> CanonicalStateAccess for BlockchainProvider<N>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
{
    fn canonical_state(&self) -> CanonicalInMemoryState<EthPrimitives> {
        self.canonical_in_memory_state()
    }
}

use crate::ordering::{PoolAssembler, ProviderExecutedChain};

/// Operator-supplied DPoS configuration (parsed from CLI/env in `main.rs`).
///
/// Not `Clone`/`Debug`: it carries the move-only cert-feed `Receiver` (single
/// consumer) and is built once then moved into `spawn_dpos`.
pub struct DposConfig {
    /// DEV/TEST-ONLY plaintext hex BLS private key file. `load_bls_keypair`
    /// rejects it on deployed networks (devnet/testnet/mainnet chain_ids);
    /// production must use `bls_keystore_path`. Mutually exclusive with it.
    pub bls_key_path: Option<PathBuf>,
    /// EIP-2335 keystore JSON for the validator BLS key. Mutually
    /// exclusive with the dev/test-only `bls_key_path`. Password is read
    /// from `bls_keystore_password_file` (mode-checked).
    pub bls_keystore_path: Option<PathBuf>,
    /// Password file for the BLS keystore — file mode must satisfy
    /// `mode & 0o077 == 0`.
    pub bls_keystore_password_file: Option<PathBuf>,
    pub peer_key_path: PathBuf,
    pub staking_config_path: PathBuf,
    /// JSON file with `[{peer_pubkey, socket}, ...]` for cold-start peer
    /// discovery.
    pub bootstrappers_path: PathBuf,
    pub p2p_port: u16,
    pub dialable: Option<SocketAddr>,
    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    pub slasher_keystore_path: Option<PathBuf>,
    pub slasher_keystore_password_file: Option<PathBuf>,
    /// DEVNET-ONLY: serve commonware consensus metrics (prometheus text) on this
    /// host port for the smoke regression suite. `None` = disabled (prod default).
    pub metrics_port: Option<u16>,
    /// Cert-feed wiring for the `consensus` RPC namespace. `None` = node does not
    /// serve the cert feed (e.g. unit tests). Set on every production node.
    pub cert_feed: Option<CertFeed>,
    /// `--dpos.follower-upstream` WS URLs. Non-empty enables the UNIFIED
    /// supervisor: the node runs a cert-follow substrate while its key is
    /// outside the committee and auto-promotes/demotes at epoch boundaries
    /// (no restarts). Empty = legacy `--dpos` (signer-or-silent-verifier).
    pub follower_upstreams: Vec<String>,
    /// DEVNET/TEST-ONLY byzantine mode string (`--dpos.byzantine`). Gated behind
    /// `dpos-devnet-byzantine`; the field does not exist in a production build.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine_mode: Option<String>,
}

/// The node-side cert-feed wiring threaded from `main.rs`: the `FeedSink` goes
/// down into the marshal (2nd `Reporter`), while the receiver + state handle drive
/// the node-side [`FeedActor`] + `consensus` RPC.
pub struct CertFeed {
    pub sink: FeedSink,
    pub rx: mpsc::UnboundedReceiver<Height>,
    pub handle: FeedStateHandle,
}

/// DPoS validator CLI flags (`--dpos.*`). Flattened into the node binary's args
/// via `#[command(flatten)]`; the cross-field clap rules below
/// (`required_if_eq("dpos", "true")`, `conflicts_with`, `requires`) resolve
/// against the merged command, so they keep working even though `--dpos` itself
/// lives in the parent args struct.
#[derive(Debug, Clone, Default, clap::Args)]
#[non_exhaustive]
pub struct DposArgs {
    /// Plaintext hex BLS private key file. DEV/TEST-ONLY — rejected at
    /// startup on deployed networks (devnet/testnet/mainnet). Production
    /// MUST use `--dpos.bls-keystore-path` (import an externally-generated
    /// EIP-2335 keystore). Mutually exclusive with the keystore flag.
    #[arg(
        long = "dpos.bls-key-path",
        env = "FLUENT_DPOS_BLS_KEY_PATH",
        conflicts_with = "dpos_bls_keystore_path",
        group = "bls"
    )]
    pub dpos_bls_key_path: Option<PathBuf>,

    /// EIP-2335 keystore JSON for the validator BLS key. Preferred over
    /// the deprecated `--dpos.bls-key-path`.
    #[arg(
        long = "dpos.bls-keystore-path",
        env = "FLUENT_DPOS_BLS_KEYSTORE_PATH",
        conflicts_with = "dpos_bls_key_path",
        requires = "dpos_bls_keystore_password_file",
        group = "bls"
    )]
    pub dpos_bls_keystore_path: Option<PathBuf>,

    /// Password file for `--dpos.bls-keystore-path`. Mode must be
    /// `0o600` (or stricter); fail-stops on world/group readable bits.
    #[arg(
        long = "dpos.bls-keystore-password-file",
        env = "FLUENT_DPOS_BLS_KEYSTORE_PASSWORD_FILE"
    )]
    pub dpos_bls_keystore_password_file: Option<PathBuf>,

    /// Path to Ed25519 peer signing key file (hex-encoded).
    #[arg(
        long = "dpos.peer-key-path",
        env = "FLUENT_DPOS_PEER_KEY_PATH",
        required_if_eq("dpos", "true")
    )]
    pub dpos_peer_key_path: Option<PathBuf>,

    /// Path to staking-reader JSON config (staking + chain-config addresses).
    #[arg(
        long = "dpos.staking-config",
        env = "FLUENT_DPOS_STAKING_CONFIG",
        required_if_eq("dpos", "true")
    )]
    pub dpos_staking_config: Option<PathBuf>,

    /// JSON file with `[{peer_pubkey, socket}, ...]` for cold-start peer
    /// discovery. Required when `--dpos` is set — no in-tree per-chain
    /// defaults. For genesis bootstrap event pass an empty `[]` JSON
    /// (explicit operator intent for the first bootnode in a new network).
    #[arg(
        long = "dpos.bootstrappers",
        env = "FLUENT_DPOS_BOOTSTRAPPERS",
        required_if_eq("dpos", "true")
    )]
    pub dpos_bootstrappers: Option<PathBuf>,

    /// Listen port for commonware p2p (default 9000).
    #[arg(
        long = "dpos.p2p-port",
        env = fluentbase_p2p::constants::LISTEN_PORT_ENV_VAR,
        default_value_t = fluentbase_p2p::constants::DEFAULT_LISTEN_PORT,
    )]
    pub dpos_p2p_port: u16,

    /// Override dialable address (what we tell peers); default = listen.
    #[arg(long = "dpos.dialable", env = "FLUENT_DPOS_DIALABLE")]
    pub dpos_dialable: Option<SocketAddr>,

    /// DEVNET-ONLY: serve commonware consensus metrics (prometheus text) on this
    /// host port for the smoke regression suite. Unset = disabled (prod default).
    #[arg(long = "dpos.metrics-port", env = "FLUENT_DPOS_METRICS_PORT")]
    pub dpos_metrics_port: Option<u16>,

    /// Upstream `consensus` WS URL(s) for the unified supervisor's follower
    /// substrate (repeatable — failover list). Presence enables unified mode:
    /// the node cert-follows while its key is outside the committee and
    /// auto-promotes to signer at its first committee epoch boundary (and
    /// demotes back on rotation-out) — no restarts. Absent = legacy `--dpos`.
    #[arg(
        long = "dpos.follower-upstream",
        env = "FLUENT_DPOS_FOLLOWER_UPSTREAM",
        action = clap::ArgAction::Append
    )]
    pub dpos_follower_upstream: Vec<String>,

    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    #[arg(
        long = "dpos.slasher-keystore-path",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PATH",
        requires = "dpos_slasher_keystore_password_file",
        required_if_eq("dpos", "true")
    )]
    pub dpos_slasher_keystore_path: Option<PathBuf>,

    /// Password file for `--dpos.slasher-keystore-path`. Mode must be
    /// `0o600` (or stricter); fail-stops on world/group readable bits.
    #[arg(
        long = "dpos.slasher-keystore-password-file",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PASSWORD_FILE"
    )]
    pub dpos_slasher_keystore_password_file: Option<PathBuf>,

    /// DEVNET/TEST-ONLY byzantine validator mode (e.g. `forge-beacon-pk`). Compiled
    /// in ONLY with the `dpos-devnet-byzantine` cargo feature; the flag does not
    /// exist in a production build. Used by the byzantine-vrf smoke to prove the
    /// beacon's safety against a forged `PK_E`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    #[arg(long = "dpos.byzantine", env = "FLUENT_DPOS_BYZANTINE")]
    pub dpos_byzantine: Option<String>,
}

impl DposConfig {
    /// Build from parsed [`DposArgs`] plus the runtime-wired `extra_data_registry`
    /// and `cert_feed`. Only reached when `--dpos` is set, so the
    /// `required_if_eq("dpos", "true")` clap rules guarantee `peer_key_path` /
    /// `staking_config_path` / `bootstrappers_path` are `Some`.
    pub fn from_args(args: &DposArgs, cert_feed: Option<CertFeed>) -> Self {
        Self {
            bls_key_path: args.dpos_bls_key_path.clone(),
            bls_keystore_path: args.dpos_bls_keystore_path.clone(),
            bls_keystore_password_file: args.dpos_bls_keystore_password_file.clone(),
            peer_key_path: args
                .dpos_peer_key_path
                .clone()
                .expect("required_if_eq guarantees --dpos.peer-key-path"),
            staking_config_path: args
                .dpos_staking_config
                .clone()
                .expect("required_if_eq guarantees --dpos.staking-config"),
            bootstrappers_path: args
                .dpos_bootstrappers
                .clone()
                .expect("required_if_eq guarantees --dpos.bootstrappers"),
            p2p_port: args.dpos_p2p_port,
            dialable: args.dpos_dialable,
            slasher_keystore_path: args.dpos_slasher_keystore_path.clone(),
            slasher_keystore_password_file: args.dpos_slasher_keystore_password_file.clone(),
            metrics_port: args.dpos_metrics_port,
            cert_feed,
            follower_upstreams: args.dpos_follower_upstream.clone(),
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine_mode: args.dpos_byzantine.clone(),
        }
    }
}

pub type DposSpawn<N, AddOns> = crate::utils::ConsensusSpawn<N, AddOns>;

/// Spawn the DPoS validator thread. The thread blocks on `handle_tx`
/// until the reth `FullNode` is delivered, then constructs the commonware
/// tokio runtime and calls [`run_dpos_stack`].
pub fn spawn_dpos<N, AddOns>(
    cfg: DposConfig,
    shutdown_token: CancellationToken,
) -> DposSpawn<N, AddOns>
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
    crate::utils::spawn_consensus_thread("dpos", move |ctx, node| async move {
        if cfg.follower_upstreams.is_empty() {
            run_dpos_stack(ctx, node, cfg, shutdown_token).await
        } else {
            crate::unified::run_unified_stack(ctx, node, cfg, shutdown_token).await
        }
    })
}

/// The DPoS thread body — runs entirely on the commonware tokio runtime.
/// Loads operator keys + JSON configs from disk, builds `PoolTxSink` from
/// `node.pool`, decomposes `FullNode` into `RethHandle`, hands everything
/// to [`DposLayer::launch`], then runs the supervisor `select!`.
async fn run_dpos_stack<N, AddOns>(
    ctx: Context,
    node: FullNode<N, AddOns>,
    mut cfg: DposConfig,
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
    spawn_devnet_metrics(&ctx, &cfg);

    let cert_feed = cfg.cert_feed.take();
    // Claim the single-execution import escrow ONCE per process.
    let beacon_engine =
        crate::importer::RethImporter::from_env(node.add_ons_handle.beacon_engine_handle.clone())?;

    // Always-on beacon plane (one FluentP2P + 5 persistent Muxers + persistent
    // DkgActor + shared store), built ONCE — the legacy `--dpos` path runs a single
    // signer engine over it (it CLONES the shared plane, including the MuxHandles).
    let mut plane = build_beacon_plane(&ctx, &node, &cfg).await?;
    let mut handle = launch_dpos_layer(
        ctx,
        &node,
        &cfg,
        beacon_engine,
        cert_feed,
        false,
        None,
        plane.shared.clone(),
        shutdown_token.clone(),
    )
    .await?;

    // Supervisor: on any unexpected exit, cancel the shared
    // shutdown_token so reth/main also bring everything down gracefully;
    // abort the surviving handles to release runtime resources.
    let exit_reason = tokio::select! {
        _ = shutdown_token.cancelled() => {
            info!("DPoS thread received shutdown signal, exiting");
            "shutdown_token"
        }
        res = &mut handle.consensus_handle => {
            match res {
                Ok(()) => warn!("OuterEngine exited cleanly (unexpected)"),
                Err(e) => error!(error = ?e, "OuterEngine task failed"),
            }
            shutdown_token.cancel();
            "consensus_exit"
        }
        res = &mut plane.net_handle => {
            match res {
                Ok(()) => warn!("p2p Network exited cleanly (unexpected)"),
                Err(e) => error!(error = ?e, "p2p Network task failed"),
            }
            shutdown_token.cancel();
            "network_exit"
        }
    };

    plane.net_handle.abort();
    plane.dkg_handle.abort();
    plane.poller_handle.abort();
    for h in &plane.mux_handles {
        h.abort();
    }
    handle.consensus_handle.abort();

    info!(reason = exit_reason, "DPoS thread exiting");
    Ok(())
}

/// DEVNET-ONLY metrics endpoint (feature-gated so prod binaries can't serve
/// it). Spawned on a child of the commonware runtime context; children share
/// the runtime's prometheus registry, so `c.encode()` includes the p2p
/// tracker `connected`/`tracked` gauges the DposLayer registers later (the
/// smoke `case-peers.sh` scrapes them). Must bind exactly ONCE per process —
/// the unified supervisor relaunches the layer per promotion, so this lives
/// outside [`launch_dpos_layer`].
pub(crate) fn spawn_devnet_metrics(ctx: &Context, cfg: &DposConfig) {
    #[cfg(feature = "dpos-devnet-metrics")]
    if let Some(port) = cfg.metrics_port {
        warn!(
            port,
            "DEVNET: serving commonware consensus metrics over HTTP (do not enable in prod)"
        );
        drop(ctx.with_label("metrics_http").spawn(move |c| async move {
            serve_metrics(c, port).await;
        }));
    }
    #[cfg(not(feature = "dpos-devnet-metrics"))]
    if cfg.metrics_port.is_some() {
        let _ = ctx;
        warn!(
            "--dpos.metrics-port set but this binary was built without the \
             `dpos-devnet-metrics` feature; metrics endpoint disabled"
        );
    }
}

/// The always-on beacon/DKG plane, built ONCE per process (before the
/// follower↔signer loop) and kept alive across every phase switch. It owns the
/// single `FluentP2P` (so there is exactly one network / `listen` bind / peer set),
/// the 5 persistent non-beacon channel `Muxer`s (the brokers the per-promotion
/// signer engine registers sub-channels against — so a demote→re-promote needs NO
/// network rebuild), the persistent `DkgActor` (committee[E] deals during E-1
/// regardless of this node's current consensus role), the shared `ceremony_store`
/// (reloaded from `<datadir>/beacon/` once), and an `EpochTransition`-driven Oracle
/// peer set fed by a finalized-height poller (the DKG clock that keeps ticking while
/// this node is a FOLLOWER). The signer engine CLONES the shared `Arc`s +
/// `MuxHandle`s per promotion; only its consensus engine is aborted on a phase
/// switch — the plane's network/broker/DkgActor handles survive until process
/// shutdown.
pub(crate) struct BeaconPlane {
    /// The single network's start handle — aborted ONLY at process shutdown.
    pub net_handle: Handle<()>,
    /// The persistent `DkgActor` task — aborted ONLY at process shutdown.
    pub dkg_handle: Handle<()>,
    /// The finalized-height poller driving the plane's ET + `dkg_height` clock.
    pub poller_handle: Handle<()>,
    /// The 5 persistent non-beacon `Muxer` broker tasks (vote/cert/resolver/
    /// broadcast/marshal) + the vote-backup forwarder — aborted ONLY at process
    /// shutdown (they outlive every per-promotion signer engine).
    pub mux_handles: Vec<Handle<()>>,
    /// Shared store + committee resolver + Oracle + metrics + the 5 `MuxHandle`s +
    /// the vote-backup forwarder, CLONED into the signer engine per promotion (the
    /// engine never re-builds any of these or re-binds the network).
    pub shared: SharedBeaconPlane,
}

/// Build the always-on beacon plane for a registered `--dpos` validator. Mirrors
/// the network/EpochTransition/DkgActor construction that used to live inside
/// `DposLayer::launch`, lifted UP so it persists across the consensus role switch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_beacon_plane<N, AddOns>(
    ctx: &Context,
    node: &FullNode<N, AddOns>,
    cfg: &DposConfig,
) -> eyre::Result<BeaconPlane>
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
    <N as FullNodeComponents>::Evm:
        reth_evm::ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    let chain_id = node.chain_spec().chain_id();
    let peer_keypair = fluentbase_p2p::read_ed25519_key_from_file(&cfg.peer_key_path)
        .wrap_err_with(|| format!("failed loading peer key from {}", cfg.peer_key_path.display()))?;
    let staking_config = fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
        &cfg.staking_config_path,
    )
    .wrap_err_with(|| {
        format!(
            "failed loading staking config from {}",
            cfg.staking_config_path.display()
        )
    })?;
    let bootstrappers = fluentbase_p2p::bootstrappers::load_from_json_path(&cfg.bootstrappers_path)
        .wrap_err_with(|| {
            format!(
                "failed loading bootstrappers from {}",
                cfg.bootstrappers_path.display()
            )
        })?;

    // The ONE network: build + start ONCE. The beacon halves go to the DkgActor;
    // the non-beacon halves are handed down to the (first) signer engine; the oracle
    // (the only Clone handle) is shared by the plane's ET and the engine's blocker.
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), cfg.p2p_port);
    let dialable = cfg.dialable.unwrap_or(listen);
    let (p2p, handles) = FluentP2P::build(
        ctx.clone(),
        FluentP2PConfig {
            crypto: peer_keypair.clone(),
            chain_id,
            listen,
            dialable: Ingress::Socket(dialable),
            bootstrappers,
        },
    );
    let net_handle = p2p.start();

    // The 5 persistent non-beacon channel Muxers. The plane owns these brokers for
    // the whole process; each promotion's signer engine CLONES the `MuxHandle`s and
    // registers per-epoch (vote/cert/resolver) / subchannel-0 (broadcast/marshal)
    // sub-channels. A demoted engine drops its `SubReceiver`s (auto-deregister); the
    // next promotion re-registers against the SAME brokers — restart-free. The vote
    // Muxer carries a backup channel for catch-up hints; the plane forwards it to the
    // currently-active engine via a re-settable forwarder (`vote_backup`).
    let mux_mailbox = 256usize;
    let (mux_vote, vote_mux, vote_backup_rx) = Muxer::builder(
        ctx.with_label("plane_vote_mux"),
        handles.vote_sender,
        handles.vote_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    let (mux_cert, cert_mux) = Muxer::new(
        ctx.with_label("plane_cert_mux"),
        handles.cert_sender,
        handles.cert_receiver,
        mux_mailbox,
    );
    let (mux_res, resolver_mux) = Muxer::new(
        ctx.with_label("plane_resolver_mux"),
        handles.resolver_sender,
        handles.resolver_receiver,
        mux_mailbox,
    );
    let (mux_bcast, broadcast_mux) = Muxer::new(
        ctx.with_label("plane_broadcast_mux"),
        handles.broadcast_sender,
        handles.broadcast_receiver,
        mux_mailbox,
    );
    let (mux_marshal, marshal_mux) = Muxer::new(
        ctx.with_label("plane_marshal_mux"),
        handles.marshal_sender,
        handles.marshal_receiver,
        mux_mailbox,
    );
    // Adopt each Muxer's run-handle under a thin shim so all plane-broker handles
    // share one `Handle<()>` shutdown-abort type (the Muxer's `start()` returns a
    // `Handle<Result<(), Error>>`; aborting the shim aborts the awaited muxer task).
    let adopt = |label: &str, h: commonware_runtime::Handle<_>| -> Handle<()> {
        ctx.with_label(label).spawn(move |_| async move {
            if let Ok(Err(e)) = h.await {
                warn!(error = ?e, "plane Muxer p2p receiver failed");
            }
        })
    };
    let mut mux_handles: Vec<Handle<()>> = vec![
        adopt("plane_vote_mux_sup", mux_vote.start()),
        adopt("plane_cert_mux_sup", mux_cert.start()),
        adopt("plane_resolver_mux_sup", mux_res.start()),
        adopt("plane_broadcast_mux_sup", mux_bcast.start()),
        adopt("plane_marshal_mux_sup", mux_marshal.start()),
    ];

    // Vote-backup re-settable forwarder: the plane owns the move-only backup
    // receiver and re-broadcasts each catch-up item to the CURRENTLY-active
    // EpochManager (`subscribe()`d fresh per promotion). While no engine is up the
    // parked sender is `None`/closed and items are dropped — a follower needs no
    // catch-up hint.
    let vote_backup: ResettableForward<VoteBackupItem> = ResettableForward::new(mux_mailbox);
    mux_handles.push({
        let slot = vote_backup.slot();
        ctx.with_label("plane_vote_backup_fwd")
            .spawn(move |_| async move {
                let mut rx = vote_backup_rx;
                while let Some(item) = rx.recv().await {
                    let guard = slot.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.try_send(item);
                    }
                }
            })
    });

    // Shared live-DKG store, reloaded from `<datadir>/beacon/` ONCE.
    let beacon_dir = node.data_dir.data_dir().join("beacon");
    let ceremony_store: fluentbase_consensus::beacon::actor::CeremonyStore =
        Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
    // Edge-trigger the DkgActor fires when a share lands; the EpochManager's
    // `enter` waits on it (instead of polling) so a signer that reaches the
    // boundary before its share is memoized wakes the instant it lands.
    let share_notify = Arc::new(tokio::sync::Notify::new());
    let reloaded = fluentbase_consensus::beacon::share_state::load_all(&beacon_dir);
    if !reloaded.is_empty() {
        if let Ok(mut store) = ceremony_store.write() {
            for (epoch, output, share) in reloaded {
                info!(epoch, "beacon: reloaded persisted live-DKG share from disk");
                store.insert(epoch, (output, share));
            }
        }
    }

    // On-chain committee resolver (deal/carry-forward set), shared by the DkgActor
    // and the per-engine verify gate. Reads committee[E] at the current finalized
    // EVM hash (mirrors the resolver that used to live in `DposLayer::launch`).
    let committee_for: fluentbase_consensus::beacon::actor::CommitteeFor = {
        let reader = RethStakingStateReader::new(
            node.provider.clone(),
            node.evm_config.clone(),
            staking_config.clone(),
        );
        let provider = node.provider.clone();
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

    // Beacon counters — registered ONCE here (the persistent layer); cloned (never
    // re-registered) into the DkgActor + each per-epoch signer engine.
    let beacon_metrics = fluentbase_consensus::beacon::metrics::BeaconMetrics::default();
    beacon_metrics.register(ctx);

    // EpochTransition-driven Oracle peer set + the `dkg_height` clock, both fed by a
    // persistent finalized-height poller (reth `finalized_block_number`) — a source
    // that exists in BOTH the follower and signer phases, unlike the per-engine
    // boundary hook. cold_start tracks the initial committee's peer set so the node
    // is connected on BEACON_CHANNEL from block 1 of its follower phase.
    let (dkg_height_tx, dkg_height_rx) = mpsc::channel::<u64>(256);
    let cache = Arc::new(Mutex::new(
        ValidatorSetCache::init(ctx.with_label("beacon_plane_cache"))
            .await
            .wrap_err("failed initializing beacon-plane ValidatorSetCache")?,
    ));
    let et_reader = RethStakingStateReader::new(
        node.provider.clone(),
        node.evm_config.clone(),
        staking_config.clone(),
    );
    let provider_for_et = node.provider.clone();
    let mut epoch_transition = EpochTransition::new(
        et_reader,
        cache,
        handles.oracle.clone(),
        fluentbase_p2p::constants::MAX_REGISTRY_PEER_SET as usize,
        None,
        Arc::new(move |n| provider_for_et.block_hash(n).ok().flatten()),
        fluentbase_consensus::K,
    );
    let (cs_fin_num, cs_fin_hash) = node
        .provider
        .finalized_block_num_hash()
        .ok()
        .flatten()
        .map(|nh| (nh.number, nh.hash))
        .unwrap_or_else(|| (0, node.chain_spec().genesis_hash()));
    epoch_transition
        .cold_start(cs_fin_hash, cs_fin_num)
        .await
        .wrap_err("beacon-plane epoch_transition cold_start failed")?;
    let et_arc = Arc::new(Mutex::new(epoch_transition));

    // Finalized-height poller, feeding TWO sinks off the SAME EL-finalized cursor:
    //   - `dkg_height` ← `fin + K` (ORDERING-finalized): the executor sets the
    //     EL-finalized height = `result_final_height(tip, floor) = ordering_finalized
    //     − K` (`order_block.rs::result_final_height`, `K = fluentbase_consensus::K`),
    //     so `fin + K` is the ordering-finalized height that produced `fin`. The
    //     DkgActor's seal deadline + `epoch_start` geometry are ORDERING-chain
    //     quantities; feeding it the raw EL-finalized `fin` would silently shorten
    //     the `DKG_MARGIN_BLOCKS` window by K (the epoch-2 boundary wedge). For every
    //     epoch ≥ 1 the cold-start floor clamp is inactive, so `fin + K` == the
    //     ordering tip exactly.
    //   - `et.on_finalized(fin)` ← raw EL-finalized `fin`: the peer-set tracker's
    //     `read_height_for(n) = n − K` contract assumes a finalized input; do NOT
    //     shift it.
    let poller_handle = {
        let provider = node.provider.clone();
        let et = et_arc.clone();
        let dkg_tx = dkg_height_tx.clone();
        ctx.with_label("beacon_plane_poller")
            .spawn(move |c| async move {
                let mut sent = cs_fin_num;
                let _ = dkg_tx.try_send(cs_fin_num + fluentbase_consensus::K);
                loop {
                    c.sleep(Duration::from_millis(500)).await;
                    let Ok(Some(fin)) = provider.finalized_block_number() else {
                        continue;
                    };
                    while sent < fin {
                        sent += 1;
                        let _ = dkg_tx.try_send(sent + fluentbase_consensus::K);
                    }
                    // Drive the boundary detection; errors here are non-fatal to the
                    // beacon plane (the engine's own ET is the authoritative boundary
                    // path) — log and keep the peer set tracking.
                    let outcome = { et.lock().await.on_finalized(fin).await };
                    if let Err(e) = outcome {
                        warn!(finalized = fin, error = ?e, "beacon plane: ET on_finalized failed");
                    }
                }
            })
    };

    // The persistent DkgActor — spawned ONCE, runs for the whole process.
    let dkg_namespace = fluentbase_consensus::beacon::seed::seed_namespace(
        &fluentbase_bls::fluent_namespace(chain_id),
    );
    let activation = {
        let reader = RethStakingStateReader::new(
            node.provider.clone(),
            node.evm_config.clone(),
            staking_config.clone(),
        );
        reader.dpos_activation_block(cs_fin_hash).unwrap_or(0)
    };
    let interval = {
        let reader = RethStakingStateReader::new(
            node.provider.clone(),
            node.evm_config.clone(),
            staking_config.clone(),
        );
        reader.epoch_block_interval(cs_fin_hash).unwrap_or(1).max(1)
    };
    let dkg_actor = fluentbase_consensus::beacon::actor::DkgActor::new(
        dkg_namespace,
        peer_keypair,
        handles.beacon_sender,
        handles.beacon_receiver,
        committee_for.clone(),
        ceremony_store.clone(),
        share_notify.clone(),
        activation,
        interval as u64,
        beacon_metrics.clone(),
        Some(beacon_dir),
    );
    let dkg_handle = ctx
        .with_label("dkg_actor")
        .spawn(move |c| async move { dkg_actor.run(dkg_height_rx, c).await });

    info!(
        listen = %listen,
        activation,
        interval,
        "always-on beacon plane built (one FluentP2P, persistent DkgActor)"
    );

    Ok(BeaconPlane {
        net_handle,
        dkg_handle,
        poller_handle,
        mux_handles,
        shared: SharedBeaconPlane {
            oracle: handles.oracle,
            ceremony_store,
            share_notify,
            committee_for,
            beacon_metrics,
            // Share each MuxHandle behind Arc<Mutex> (the per-promotion Clone + the
            // transient register-lock; the move-only DiscReceiver makes the bare
            // MuxHandle un-Clone-able).
            vote_mux: Arc::new(Mutex::new(vote_mux)),
            cert_mux: Arc::new(Mutex::new(cert_mux)),
            resolver_mux: Arc::new(Mutex::new(resolver_mux)),
            broadcast_mux: Arc::new(Mutex::new(broadcast_mux)),
            marshal_mux: Arc::new(Mutex::new(marshal_mux)),
            vote_backup,
        },
    })
}

/// Build and launch the DPoS layer once: load operator keys + JSON configs,
/// construct the `PoolTxSink`/deriver/assembler over the node's own
/// provider, hand everything to [`DposLayer::launch`], wire the cert-feed
/// actor and `set_marshal`. Extracted from [`run_dpos_stack`] so the unified
/// supervisor can relaunch the signer stack per promotion; `promotion` /
/// `mode_events` are the supervisor inputs (legacy passes `false` / `None`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn launch_dpos_layer<N, AddOns>(
    ctx: Context,
    node: &FullNode<N, AddOns>,
    cfg: &DposConfig,
    beacon_engine: crate::importer::RethImporter,
    cert_feed: Option<CertFeed>,
    promotion: bool,
    mode_events: Option<tokio::sync::mpsc::UnboundedSender<fluentbase_consensus::ModeEvent>>,
    shared_beacon: SharedBeaconPlane,
    shutdown_token: CancellationToken,
) -> eyre::Result<DposLayerHandle>
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
    let bls_keypair = load_bls_keypair(cfg, chain_id)?;
    let peer_keypair = fluentbase_p2p::read_ed25519_key_from_file(&cfg.peer_key_path)
        .wrap_err_with(|| {
            format!(
                "failed loading peer key from {}",
                cfg.peer_key_path.display()
            )
        })?;
    let slasher_signer = load_slasher_signer(cfg)?;

    let staking_config = fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
        &cfg.staking_config_path,
    )
    .wrap_err_with(|| {
        format!(
            "failed loading staking config from {}",
            cfg.staking_config_path.display()
        )
    })?;

    // Build PoolTxSink host-side: PoolTxSink<P, Provider> carries concrete
    // reth-transaction-pool trait bounds (PoolTransaction<Consensus =
    // EthereumTxEnvelope<TxEip4844>>) that can't compile in the consensus
    // crate, so this construction stays here.
    let slasher_sink: Arc<dyn fluentbase_consensus::slasher::actor::SlasherTxSink> =
        Arc::new(crate::slasher_sink::PoolTxSink::new(
            slasher_signer,
            chain_id,
            node.pool.clone(),
            node.provider.clone(),
            node.evm_config.clone(),
        ));

    let canonical_state = node.provider.canonical_state();
    let genesis_hash = node.chain_spec().genesis_hash();
    let reth = RethHandle {
        provider: node.provider.clone(),
        evm_config: node.evm_config.clone(),
        beacon_engine_handle: beacon_engine,
        chain_id,
        canonical_state,
        genesis_hash,
    };

    // Deferred-execution collaborators, all over the node's own provider/EVM:
    // derive (reth-evm BlockBuilder), the derived-chain view, and the
    // pool-backed ordering assembler. The beacon is always-on live-DKG; the
    // deriver computes prev_randao = H(seed) directly from the cert-recovered
    // seed (no on-chain PK_E read — that layer is gone, DPOS_ARCHITECTURE §8.11).
    let deriver =
        crate::derive::RethBlockDeriver::new(node.provider.clone(), node.evm_config.clone());
    let executed = ProviderExecutedChain(node.provider.clone());
    let assembler = Arc::new(PoolAssembler::new(node.pool.clone(), executed.clone()));
    // The protocol fee manager — same recipient the pre-deferred attrs
    // builder used; uniform across honest nodes (agreed data once embedded).
    let fee_recipient = fluentbase_types::PRECOMPILE_FEE_MANAGER;
    // Gas-limit target = the chain's genesis gas limit (protocol default; the
    // EIP-1559 ±1/1024 step walks the agreed limit toward it).
    let target_gas_limit = node.chain_spec().genesis().gas_limit;

    // Cert-feed: the FeedSink goes DOWN into the marshal as its 2nd Reporter; the
    // receiver + state handle stay here to drive the node-side feed actor + RPC.
    let (feed_sink, feed_actor_wiring) = match cert_feed {
        Some(cf) => (Some(cf.sink), Some((cf.rx, cf.handle))),
        None => (None, None),
    };

    // DEVNET/TEST-ONLY: parse the byzantine mode string. An unknown value
    // fails-loud rather than silently running honest (a misconfigured smoke must
    // not false-pass). The whole block is gated out of production builds.
    #[cfg(feature = "dpos-devnet-byzantine")]
    let byzantine = match cfg.byzantine_mode.as_deref() {
        None => None,
        Some("forge-beacon-pk") => {
            tracing::warn!("DEVNET BYZANTINE MODE ACTIVE: forge-beacon-pk — NEVER use in production");
            Some(fluentbase_consensus::application::ByzantineMode::ForgeBeaconPk)
        }
        Some("equivocate") => {
            tracing::warn!("DEVNET BYZANTINE MODE ACTIVE: equivocate — NEVER use in production");
            Some(fluentbase_consensus::application::ByzantineMode::Equivocate)
        }
        Some(other) => {
            eyre::bail!(
                "unknown --dpos.byzantine mode: {other:?} \
                 (expected `forge-beacon-pk` or `equivocate`)"
            )
        }
    };

    let layer_cfg = DposLayerConfig {
        bls_keypair,
        peer_keypair,
        slasher_sink,
        staking_config,
        deriver,
        executed,
        assembler,
        fee_recipient,
        target_gas_limit,
        feed: feed_sink,
        promotion,
        mode_events,
        // The always-on beacon plane (shared store + committee resolver + the single
        // network's oracle + the once-registered metrics + the 5 non-beacon MuxHandles
        // + the vote-backup forwarder). The signer engine CLONES these per promotion;
        // it never rebuilds the network, re-spawns the DkgActor, or consumes a raw
        // channel half — so a demote→re-promote re-clones with no rebuild.
        beacon_plane: shared_beacon,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byzantine,
    };

    // Spawn the cert-feed actor on a child of the runtime context BEFORE `launch`
    // consumes `ctx`. It blocks on the channel until finalizations flow (post-launch),
    // by which point `set_marshal` (below) has run. Keep the handle for `set_marshal`.
    let feed_handle = feed_actor_wiring.map(|(rx, handle)| {
        let actor_handle = handle.clone();
        drop(ctx.with_label("cert_feed").spawn(move |_| async move {
            FeedActor::new(rx, actor_handle).run().await;
        }));
        handle
    });

    let handle: DposLayerHandle = DposLayer::launch(ctx, reth, layer_cfg, shutdown_token).await?;

    // Hand the marshal mailbox to the feed state (node-side, respecting the crate
    // boundary — consensus never names node types). Until this runs the RPC returns
    // ServiceUnavailable; the window is sub-finalization so no event is lost.
    if let Some(fh) = feed_handle {
        fh.set_marshal(handle.cert_mailbox.clone());
    }

    Ok(handle)
}

/// DEVNET-ONLY: minimal HTTP/1.0 responder serving the commonware runtime's
/// prometheus metrics (`ctx.encode()`) on every request. Uses `tokio::net` (not
/// `std::net`) so the blocking accept never starves the shared async executor.
#[cfg(feature = "dpos-devnet-metrics")]
async fn serve_metrics(ctx: Context, port: u16) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(error = ?e, port, "metrics_http: bind failed; metrics endpoint disabled");
            return;
        }
    };
    loop {
        let mut sock = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                warn!(error = ?e, "metrics_http: accept failed");
                continue;
            }
        };
        let mut scratch = [0u8; 1024];
        let _ = sock.read(&mut scratch).await; // drain the request; we ignore it
        let body = ctx.encode();
        let resp = format!(
            "HTTP/1.0 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Host-only key loading helpers (filesystem syscalls + permission checks)
// ───────────────────────────────────────────────────────────────────────────

fn load_bls_keypair(
    cfg: &DposConfig,
    chain_id: u64,
) -> eyre::Result<fluentbase_bls::keys::ValidatorBlsKeypair> {
    match (
        cfg.bls_keystore_path.as_deref(),
        cfg.bls_key_path.as_deref(),
    ) {
        (Some(keystore_path), None) => {
            let password_path = cfg.bls_keystore_password_file.as_deref().ok_or_eyre(
                "--dpos.bls-keystore-path requires --dpos.bls-keystore-password-file",
            )?;
            let password = read_password_file(password_path, "BLS keystore")?;
            fluentbase_bls::keys::ValidatorBlsKeypair::read_from_keystore(
                keystore_path,
                password.trim().as_bytes(),
            )
            .wrap_err_with(|| {
                format!(
                    "failed loading BLS keystore from {}",
                    keystore_path.display()
                )
            })
        }
        (None, Some(plain_path)) => {
            use crate::chainspec::{
                FLUENT_DEVNET_CHAIN_ID, FLUENT_MAINNET_CHAIN_ID, FLUENT_TESTNET_CHAIN_ID,
            };
            if matches!(
                chain_id,
                FLUENT_DEVNET_CHAIN_ID | FLUENT_TESTNET_CHAIN_ID | FLUENT_MAINNET_CHAIN_ID
            ) {
                return Err(eyre!(
                    "--dpos.bls-key-path (plaintext BLS key) is forbidden on deployed network \
                     (chain_id {chain_id}); production must use --dpos.bls-keystore-path with an \
                     EIP-2335 keystore"
                ));
            }
            info!(chain_id, path = %plain_path.display(), "loading dev/test plaintext BLS key");
            check_mode_600(plain_path).wrap_err("plaintext BLS key file mode check")?;
            fluentbase_bls::keys::ValidatorBlsKeypair::read_from_file(plain_path)
                .wrap_err_with(|| format!("failed loading BLS key from {}", plain_path.display()))
        }
        _ => Err(eyre!(
            "exactly one of --dpos.bls-keystore-path | --dpos.bls-key-path must be set"
        )),
    }
}

fn load_slasher_signer(cfg: &DposConfig) -> eyre::Result<alloy_signer_local::PrivateKeySigner> {
    let keystore_path = cfg.slasher_keystore_path.as_deref().ok_or_eyre(
        "--dpos.slasher-keystore-path is required (with --dpos.slasher-keystore-password-file)",
    )?;
    let password_path = cfg.slasher_keystore_password_file.as_deref().ok_or_eyre(
        "--dpos.slasher-keystore-path requires --dpos.slasher-keystore-password-file",
    )?;
    let password = read_password_file(password_path, "slasher keystore")?;
    alloy_signer_local::LocalSigner::decrypt_keystore(keystore_path, password.trim())
        .map_err(|e| eyre!("failed decrypting slasher keystore: {e}"))
}

/// Read a keystore password file: enforce 0600 (or stricter) mode, then read into
/// a zeroizing buffer cleared on drop. `what` labels the file in error messages.
fn read_password_file(
    path: &std::path::Path,
    what: &str,
) -> eyre::Result<zeroize::Zeroizing<String>> {
    check_mode_600(path).wrap_err_with(|| format!("{what} password file mode check"))?;
    Ok(zeroize::Zeroizing::new(
        std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed reading {what} password from {}", path.display()))?,
    ))
}

#[cfg(unix)]
fn check_mode_600(path: &std::path::Path) -> eyre::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(eyre!(
            "{} has insecure permissions (mode 0o{:03o}); chmod 600",
            path.display(),
            mode & 0o777,
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_mode_600(_path: &std::path::Path) -> eyre::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chainspec::{
        FLUENT_DEVNET_CHAIN_ID, FLUENT_MAINNET_CHAIN_ID, FLUENT_TESTNET_CHAIN_ID,
    };

    fn cfg_with_plaintext_bls(path: &str) -> DposConfig {
        DposConfig {
            bls_key_path: Some(PathBuf::from(path)),
            bls_keystore_path: None,
            bls_keystore_password_file: None,
            peer_key_path: PathBuf::new(),
            staking_config_path: PathBuf::new(),
            bootstrappers_path: PathBuf::new(),
            p2p_port: 0,
            dialable: None,
            slasher_keystore_path: None,
            slasher_keystore_password_file: None,
            metrics_port: None,
            cert_feed: None,
            follower_upstreams: vec![],
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine_mode: None,
        }
    }

    #[test]
    fn plaintext_bls_rejected_on_deployed_networks() {
        for cid in [
            FLUENT_DEVNET_CHAIN_ID,
            FLUENT_TESTNET_CHAIN_ID,
            FLUENT_MAINNET_CHAIN_ID,
        ] {
            let cfg = cfg_with_plaintext_bls("/nonexistent/bls.hex");
            let err = load_bls_keypair(&cfg, cid).unwrap_err().to_string();
            assert!(
                err.contains("forbidden on deployed network"),
                "chain_id {cid}: expected deployed-network rejection, got: {err}"
            );
        }
    }

    #[test]
    fn plaintext_bls_gate_bypassed_on_local_network() {
        // chain_id 1337 (localnet) is not in the deployed set, so the gate
        // must NOT fire; loading then proceeds to file I/O and fails there
        // (nonexistent path) — proving the rejection is chain_id-scoped.
        let cfg = cfg_with_plaintext_bls("/nonexistent/bls.hex");
        let err = load_bls_keypair(&cfg, 1337).unwrap_err().to_string();
        assert!(
            !err.contains("forbidden on deployed network"),
            "local chain_id must bypass the deployed-network gate, got: {err}"
        );
    }
}
