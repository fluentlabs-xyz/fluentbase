//! DPoS host adapter: spawns the dedicated OS thread + commonware-tokio
//! runtime, loads operator keys and configs from disk, builds the
//! `RethHandle` + `DposLayerConfig`, calls
//! [`fluentbase_consensus::dpos::DposLayer::launch`], then runs the
//! shutdown supervisor `select!`.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    thread,
};

use commonware_runtime::{
    tokio::{Config as RuntimeConfig, Context, Runner as TokioRunner},
    Runner as _,
};
// `Metrics` (ctx.with_label, ctx.encode) + `Spawner` (ctx.spawn) — used by the
// cert-feed actor (unconditional) and the feature-gated devnet metrics endpoint.
use commonware_consensus::types::Height;
use commonware_runtime::{Metrics as _, Spawner as _};
use eyre::{eyre, OptionExt as _, WrapErr as _};
use fluentbase_consensus::dpos::{
    DposLayer, DposLayerConfig, DposLayerHandle, P2pParams, RethHandle,
};
pub use fluentbase_consensus::FeedSink;

use crate::consensus_rpc::{feed_actor::FeedActor, FeedStateHandle};
use reth_chain_state::CanonicalInMemoryState;
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode};
use reth_provider::providers::{BlockchainProvider, ProviderNodeTypes};
use reth_storage_api::{BlockHashReader, BlockIdReader, BlockNumReader, BlockReader};
use tokio::sync::{mpsc, oneshot};
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

use crate::payload::FluentPayloadAttributesBuilder;

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
    /// Shared `extra_data` registry — same `Arc<DashMap>` instance is also
    /// passed to `FluentNode::with_extra_data_registry` so payload-builder
    /// (reader) and `OuterBuilder`/`FluentApp::propose` (writer) see the
    /// same map.
    pub extra_data_registry:
        Arc<dashmap::DashMap<alloy_rpc_types_engine::PayloadId, alloy_primitives::Bytes>>,
    /// DEVNET-ONLY: serve commonware consensus metrics (prometheus text) on this
    /// host port for the smoke regression suite. `None` = disabled (prod default).
    pub metrics_port: Option<u16>,
    /// Cert-feed wiring for the `consensus` RPC namespace. `None` = node does not
    /// serve the cert feed (e.g. unit tests). Set on every production node.
    pub cert_feed: Option<CertFeed>,
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
        conflicts_with = "dpos_bls_keystore_path"
    )]
    pub dpos_bls_key_path: Option<PathBuf>,

    /// EIP-2335 keystore JSON for the validator BLS key. Preferred over
    /// the deprecated `--dpos.bls-key-path`.
    #[arg(
        long = "dpos.bls-keystore-path",
        env = "FLUENT_DPOS_BLS_KEYSTORE_PATH",
        conflicts_with = "dpos_bls_key_path",
        requires = "dpos_bls_keystore_password_file"
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

    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    #[arg(
        long = "dpos.slasher-keystore-path",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PATH",
        requires = "dpos_slasher_keystore_password_file"
    )]
    pub dpos_slasher_keystore_path: Option<PathBuf>,

    /// Password file for `--dpos.slasher-keystore-path`. Mode must be
    /// `0o600` (or stricter); fail-stops on world/group readable bits.
    #[arg(
        long = "dpos.slasher-keystore-password-file",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PASSWORD_FILE"
    )]
    pub dpos_slasher_keystore_password_file: Option<PathBuf>,
}

impl DposConfig {
    /// Build from parsed [`DposArgs`] plus the runtime-wired `extra_data_registry`
    /// and `cert_feed`. Only reached when `--dpos` is set, so the
    /// `required_if_eq("dpos", "true")` clap rules guarantee `peer_key_path` /
    /// `staking_config_path` / `bootstrappers_path` are `Some`.
    pub fn from_args(
        args: &DposArgs,
        extra_data_registry: Arc<
            dashmap::DashMap<alloy_rpc_types_engine::PayloadId, alloy_primitives::Bytes>,
        >,
        cert_feed: Option<CertFeed>,
    ) -> Self {
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
            extra_data_registry,
            metrics_port: args.dpos_metrics_port,
            cert_feed,
        }
    }
}

pub struct DposSpawn<N, AddOns>
where
    N: FullNodeComponents,
    AddOns: RethRpcAddOns<N>,
{
    pub join: thread::JoinHandle<eyre::Result<()>>,
    pub handle_tx: oneshot::Sender<FullNode<N, AddOns>>,
    pub dead_rx: oneshot::Receiver<()>,
}

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
    <N as FullNodeComponents>::Evm: Clone + Send + Sync + 'static,
{
    let (handle_tx, handle_rx) = oneshot::channel::<FullNode<N, AddOns>>();
    let (dead_tx, dead_rx) = oneshot::channel::<()>();
    let shutdown_token_inner = shutdown_token.clone();

    let join = thread::Builder::new()
        .name("dpos".into())
        .spawn(move || {
            let node = handle_rx
                .blocking_recv()
                .wrap_err("channel closed before reth FullNode could be received")?;

            let consensus_storage = node.data_dir.data_dir().join("dpos");
            info!(
                path = %consensus_storage.display(),
                "DPoS storage directory determined"
            );

            let catch_panics = true;
            let tcp_nodelay = Some(true);
            let runtime_config = RuntimeConfig::default()
                .with_tcp_nodelay(tcp_nodelay)
                .with_storage_directory(consensus_storage)
                .with_catch_panics(catch_panics);
            info!(catch_panics, ?tcp_nodelay, "DPoS RuntimeConfig");

            let runner = TokioRunner::new(runtime_config);
            let ret = runner.start(|ctx| async move {
                run_dpos_stack(ctx, node, cfg, shutdown_token_inner).await
            });
            let _ = dead_tx.send(());
            ret
        })
        .expect("failed to spawn dpos thread");

    DposSpawn {
        join,
        handle_tx,
        dead_rx,
    }
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
    <N as FullNodeComponents>::Evm: Clone + Send + Sync + 'static,
{
    let chain_id = node.chain_spec().chain_id();
    let bls_keypair = load_bls_keypair(&cfg, chain_id)?;
    let peer_keypair = fluentbase_p2p::read_ed25519_key_from_file(&cfg.peer_key_path)
        .wrap_err_with(|| {
            format!(
                "failed loading peer key from {}",
                cfg.peer_key_path.display()
            )
        })?;
    let slasher_signer = load_slasher_signer(&cfg)?;

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
    info!(
        count = bootstrappers.len(),
        path = %cfg.bootstrappers_path.display(),
        chain_id,
        "DPoS bootstrappers loaded"
    );

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

    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), cfg.p2p_port);
    let canonical_state = node.provider.canonical_state();
    let genesis_hash = node.chain_spec().genesis_hash();
    let reth = RethHandle {
        provider: node.provider.clone(),
        evm_config: node.evm_config.clone(),
        payload_builder_handle: node.payload_builder_handle.clone(),
        beacon_engine_handle: node.add_ons_handle.beacon_engine_handle.clone(),
        chain_id,
        canonical_state,
        genesis_hash,
    };

    // Cert-feed: the FeedSink goes DOWN into the marshal as its 2nd Reporter; the
    // receiver + state handle stay here to drive the node-side feed actor + RPC.
    let (feed_sink, feed_actor_wiring) = match cfg.cert_feed.take() {
        Some(cf) => (Some(cf.sink), Some((cf.rx, cf.handle))),
        None => (None, None),
    };

    let layer_cfg = DposLayerConfig {
        bls_keypair,
        peer_keypair,
        slasher_sink,
        staking_config,
        bootstrappers,
        p2p: P2pParams {
            listen,
            dialable: cfg.dialable,
        },
        payload_attrs_builder: FluentPayloadAttributesBuilder,
        extra_data_registry: cfg.extra_data_registry,
        feed: feed_sink,
    };

    // DEVNET-ONLY metrics endpoint (feature-gated so prod binaries can't serve
    // it). Spawned on a child of the commonware runtime context BEFORE `launch`
    // consumes `ctx`; children share the runtime's prometheus registry, so
    // `c.encode()` includes the p2p tracker `connected`/`tracked` gauges the
    // DposLayer registers later (the smoke `case-peers.sh` scrapes them).
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
        warn!(
            "--dpos.metrics-port set but this binary was built without the \
             `dpos-devnet-metrics` feature; metrics endpoint disabled"
        );
    }

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

    let mut handle: DposLayerHandle =
        DposLayer::launch(ctx, reth, layer_cfg, shutdown_token.clone()).await?;

    // Hand the marshal mailbox to the feed state (node-side, respecting the crate
    // boundary — consensus never names node types). Until this runs the RPC returns
    // ServiceUnavailable; the window is sub-finalization so no event is lost.
    if let Some(fh) = feed_handle {
        fh.set_marshal(handle.cert_mailbox.clone());
    }

    // Supervisor: on any unexpected exit, cancel the shared
    // shutdown_token so reth/main also bring everything down gracefully;
    // abort the surviving handle to release runtime resources.
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
        res = &mut handle.network_handle => {
            match res {
                Ok(()) => warn!("p2p Network exited cleanly (unexpected)"),
                Err(e) => error!(error = ?e, "p2p Network task failed"),
            }
            shutdown_token.cancel();
            "network_exit"
        }
    };

    handle.network_handle.abort();
    handle.consensus_handle.abort();

    info!(reason = exit_reason, "DPoS thread exiting");
    Ok(())
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
            check_mode_600(password_path).wrap_err("BLS keystore password file mode check")?;
            let password = std::fs::read_to_string(password_path).wrap_err_with(|| {
                format!(
                    "failed reading BLS keystore password from {}",
                    password_path.display()
                )
            })?;
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
    check_mode_600(password_path).wrap_err("slasher keystore password file mode check")?;
    let password = std::fs::read_to_string(password_path).wrap_err_with(|| {
        format!(
            "failed reading slasher keystore password from {}",
            password_path.display()
        )
    })?;
    alloy_signer_local::LocalSigner::decrypt_keystore(keystore_path, password.trim())
        .map_err(|e| eyre!("failed decrypting slasher keystore: {e}"))
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
            extra_data_registry: Arc::new(dashmap::DashMap::new()),
            metrics_port: None,
            cert_feed: None,
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
