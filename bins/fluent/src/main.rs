#![allow(missing_docs, dead_code)]

use alloy_primitives::Bytes;
use alloy_rpc_types_engine::PayloadId;
use clap::{Args, Parser};
use dashmap::DashMap;
use eyre::OptionExt;
use fluentbase_node::{
    cert_follow::{spawn_cert_follower, CertFollowerConfig},
    chainspec::FluentChainSpecParser,
    consensus::FluentConsensus,
    consensus_rpc::{ConsensusApiServer, ConsensusRpc, FeedStateHandle},
    dpos::{spawn_dpos, CertFeed, DposConfig, FeedSink},
    evm::{FluentEvmConfig, FluentExecutorBuilder, FluentNode},
    launcher::{launch_consensus_node, launch_consensus_validator},
    payload::FluentPayloadAttributesBuilder,
    trusted_peers::{resolve_default_consensus_url, resolve_default_trusted_peers},
};
use humantime::parse_duration;
use reth_chainspec::ChainSpec;
use reth_cli_commands::download::DownloadDefaults;
use reth_ethereum_cli::{Cli, Commands};
use reth_node_builder::{rpc::EngineShutdown, DebugNodeLauncherFuture, Node};
use reth_node_core::version::{default_reth_version_metadata, try_init_version_metadata};
use reth_node_ethereum::EthereumAddOns;
use reth_storage_api::{BlockNumReader, HeaderProvider};
use std::{borrow::Cow, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// `consensus_subscribe` broadcast buffer (slow consumers lag, not block).
const CERT_FEED_EVENT_CAP: usize = 1024;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

#[cfg(all(feature = "jemalloc-prof", unix))]
#[unsafe(export_name = "_rjem_malloc_conf")]
static MALLOC_CONF: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

fn init_fluent_version_metadata() {
    let mut meta = default_reth_version_metadata();

    let version = env!("CARGO_PKG_VERSION");
    let version_suffix = option_env!("FLUENT_VERSION_SUFFIX").unwrap_or("");
    let git_sha = option_env!("FLUENT_GIT_SHA").unwrap_or("unknown");
    let git_sha_short = option_env!("FLUENT_GIT_SHA_SHORT").unwrap_or("unknown");
    let git_tag = option_env!("FLUENT_GIT_TAG").unwrap_or("untagged");
    let build_timestamp = option_env!("FLUENT_BUILD_TIMESTAMP").unwrap_or("unknown");
    let target_triple = option_env!("FLUENT_CARGO_TARGET_TRIPLE").unwrap_or("unknown");
    let build_features = option_env!("FLUENT_CARGO_FEATURES").unwrap_or("none");
    let build_profile = option_env!("FLUENT_BUILD_PROFILE").unwrap_or("unknown");

    let short_version = format!("{version}{version_suffix} ({git_sha_short})");
    let long_version = format!(
        "Version: {version}{version_suffix}\nTag: {git_tag}\nCommit SHA: {git_sha}\nBuild Timestamp: {build_timestamp}\nBuild Features: {build_features}\nBuild Profile: {build_profile}\nTarget: {target_triple}"
    );

    let mut extra_data = format!("fluent/v{version}/{}", std::env::consts::OS);
    if extra_data.len() > 32 {
        extra_data.truncate(32);
    }

    meta.name_client = Cow::Borrowed("Fluentbase");
    meta.cargo_pkg_version = Cow::Borrowed(version);
    meta.vergen_git_sha_long = Cow::Borrowed(git_sha);
    meta.vergen_git_sha = Cow::Borrowed(git_sha_short);
    meta.vergen_build_timestamp = Cow::Borrowed(build_timestamp);
    meta.vergen_cargo_target_triple = Cow::Borrowed(target_triple);
    meta.vergen_cargo_features = Cow::Borrowed(build_features);
    meta.short_version = Cow::Owned(short_version);
    meta.long_version = Cow::Owned(long_version);
    meta.build_profile_name = Cow::Borrowed(build_profile);
    meta.p2p_client_version =
        Cow::Owned(format!("fluent/v{version}-{git_sha_short}/{target_triple}"));
    meta.extra_data = Cow::Owned(extra_data);

    let _ = try_init_version_metadata(meta);
}

#[derive(Debug, Clone, Default, Args)]
#[non_exhaustive]
pub struct FluentNodeArgs {
    #[arg(long = "validator", default_value_t = false)]
    pub validator: bool,

    #[arg(
        long = "validator.block-time",
        value_parser = parse_duration,
        default_value = "1s",
    )]
    pub validator_block_time: Duration,

    #[arg(long = "sequencer-url")]
    pub sequencer_url: Option<String>,

    /// Run as a DPoS validator (BFT consensus + p2p + finality-gated peer set).
    /// Mutually exclusive with --validator and --sequencer-url.
    #[arg(
        long = "dpos",
        default_value_t = false,
        conflicts_with_all = &["validator", "sequencer_url"],
    )]
    pub dpos: bool,

    /// Run as a trustless cert-follower: pull finality certs from
    /// `--sequencer-url` (a `consensus`-RPC WebSocket), verify each against the
    /// on-chain epoch committee, and drive this node's own reth. Mutually
    /// exclusive with `--dpos`/`--validator`; requires `--sequencer-url` (the
    /// upstream WS) and `--dpos.staking-config` (committee reads).
    #[arg(
        long = "cert-follow",
        default_value_t = false,
        conflicts_with_all = &["dpos", "validator"],
        requires_all = &["sequencer_url", "dpos_staking_config"],
    )]
    pub cert_follow: bool,

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

/// Trigger reth's engine shutdown and await persistence of in-memory blocks
/// to MDBX. Idempotent: `shutdown()` yields `Some(done_rx)` only on the first
/// call; the later of the two flush sites (signal watcher vs DPoS-death path)
/// observes `None` and skips.
async fn flush_engine_in_memory(engine_shutdown: &EngineShutdown, ctx: &str) {
    if let Some(done_rx) = engine_shutdown.shutdown() {
        match done_rx.await {
            Ok(()) => info!(ctx, "engine shutdown complete (in-memory blocks persisted)"),
            Err(_) => error!(ctx, "engine_shutdown done_tx dropped"),
        }
    } else {
        info!(ctx, "engine_shutdown already triggered (race); skipping");
    }
}

fn init_downloads_defaults() {
    let download_defaults = DownloadDefaults {
        available_snapshots: vec![Cow::Borrowed(
            "https://cdn.fluent.xyz/snapshots/20994/fluent-testnet-22459308.tar.gz",
        )],
        default_base_url: Cow::Borrowed("https://cdn.fluent.xyz/snapshots"),
        default_chain_aware_base_url: Some(Cow::Borrowed("https://cdn.fluent.xyz/snapshots")),
        long_help: None,
    };
    download_defaults
        .try_init()
        .expect("failed to initialize download URLs");
}

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    // Initialize default download URLs for snapshots
    init_downloads_defaults();

    // Override default reth version metadata with fluentbase-specific build metadata.
    init_fluent_version_metadata();

    let mut consensus_url: Option<String> = None;
    let mut block_producer: Option<Duration> = None;
    let mut dpos_config: Option<DposConfig> = None;
    // Set when `--cert-follow` is passed: routes to the trustless cert-follower
    // thread instead of the trust-follow block relay (`launch_consensus_node`).
    let mut cert_follow_config: Option<CertFollowerConfig> = None;
    // Cert-feed state handle, shared into the `consensus` RPC closure below. Set
    // alongside `dpos_config` when DPoS is enabled (the node serves the cert feed).
    let mut cert_rpc_feed: Option<FeedStateHandle> = None;
    // Pipeline 2 (Tempo→DPoS migration): parsed from
    // `--dpos.staking-config` independent of `--dpos`. Tempo and
    // follower modes need non-zero addresses so the executor's
    // `commitEpochCommittee` system call fires at epoch boundaries —
    // required for prod migration past the first epoch boundary.
    let mut staking_address = alloy_primitives::Address::ZERO;
    let mut chain_config_address = alloy_primitives::Address::ZERO;
    // Retained to build the activation-block reader for the Tempo sequencer's
    // production gate (clean-halt at `dposActivationBlock`).
    let mut staking_reader_cfg: Option<fluentbase_staking_reader::reader::StakingReaderConfig> =
        None;

    // Single shared `Arc<DashMap>` instance
    // between `FluentNode`'s payload builder (reader) and the DPoS thread's
    // `OuterBuilder.extra_data_registry` (writer). Empty in non-DPoS modes
    // — the executor's `processBitmap` system call no-ops on
    // `committeeSize == 0` decoded from an empty extra_data.
    let extra_data_registry: Arc<DashMap<PayloadId, Bytes>> = Arc::new(DashMap::new());

    let mut cli = Cli::<FluentChainSpecParser, FluentNodeArgs>::parse();

    // Adjust several params for node execution
    if let Commands::Node(node) = &mut cli.command {
        // Merge default public trusted peers
        let new_trusted_peers = resolve_default_trusted_peers(node.chain.chain);
        node.network.trusted_peers.extend(new_trusted_peers);

        // If consensus URL is not specified, resolve default
        if let Some(sequencer_url) = &node.ext.sequencer_url {
            consensus_url = Some(sequencer_url.clone());
        } else if let Some(debug_consensus_url) = &node.debug.rpc_consensus_url {
            consensus_url = Some(debug_consensus_url.clone());
        } else {
            consensus_url = resolve_default_consensus_url(node.chain.chain);
        }

        // If validator mode is enabled then specify block production time
        if node.ext.validator {
            block_producer = Some(node.ext.validator_block_time);
        }

        // If DPoS mode is enabled, build the DposConfig from required args.
        // `required_if_eq("dpos", "true")` on the underlying clap fields
        // guarantees the `Option`s are `Some` here.
        if node.ext.dpos {
            // Cert-feed wiring: the FeedSink is the marshal's 2nd Reporter; the
            // handle is shared with the `consensus` RPC; the receiver drives the
            // feed actor (spawned inside `run_dpos_stack`).
            let feed_handle = FeedStateHandle::new(CERT_FEED_EVENT_CAP);
            let (feed_sink, feed_rx) = FeedSink::channel();
            cert_rpc_feed = Some(feed_handle.clone());
            dpos_config = Some(DposConfig {
                bls_key_path: node.ext.dpos_bls_key_path.clone(),
                bls_keystore_path: node.ext.dpos_bls_keystore_path.clone(),
                bls_keystore_password_file: node.ext.dpos_bls_keystore_password_file.clone(),
                peer_key_path: node
                    .ext
                    .dpos_peer_key_path
                    .clone()
                    .expect("required_if_eq guarantees --dpos.peer-key-path"),
                staking_config_path: node
                    .ext
                    .dpos_staking_config
                    .clone()
                    .expect("required_if_eq guarantees --dpos.staking-config"),
                bootstrappers_path: node
                    .ext
                    .dpos_bootstrappers
                    .clone()
                    .expect("required_if_eq guarantees --dpos.bootstrappers"),
                p2p_port: node.ext.dpos_p2p_port,
                dialable: node.ext.dpos_dialable,
                slasher_keystore_path: node.ext.dpos_slasher_keystore_path.clone(),
                slasher_keystore_password_file: node
                    .ext
                    .dpos_slasher_keystore_password_file
                    .clone(),
                extra_data_registry: extra_data_registry.clone(),
                metrics_port: node.ext.dpos_metrics_port,
                cert_feed: Some(CertFeed {
                    sink: feed_sink,
                    rx: feed_rx,
                    handle: feed_handle,
                }),
            });
        }

        // Cert-follower mode: drive reth from upstream-verified certs instead of
        // the trust-follow block relay. `requires_all` (clap) guarantees both
        // `--sequencer-url` and `--dpos.staking-config` are present. Clear
        // `consensus_url` so `launch_consensus_node` (the trust path) does not
        // also run — the follower drives reth itself.
        if node.ext.cert_follow {
            cert_follow_config = Some(CertFollowerConfig {
                sequencer_url: node
                    .ext
                    .sequencer_url
                    .clone()
                    .expect("requires_all guarantees --sequencer-url"),
                staking_config_path: node
                    .ext
                    .dpos_staking_config
                    .clone()
                    .expect("requires_all guarantees --dpos.staking-config"),
            });
            consensus_url = None;
        }

        // Tempo→DPoS migration: parse
        // `--dpos.staking-config` independently of `--dpos`. Tempo
        // and follower modes also need non-zero
        // `staking_address` / `chain_config_address` so the
        // existing `commitEpochCommittee` system call in
        // `FluentBlockExecutor::apply_pre_execution_changes`
        // ([crates/node/src/evm.rs:848](../../../crates/node/src/evm.rs#L848))
        // fires at epoch boundaries. Without this, post-swap DPoS
        // validators reading
        // `epoch_committee_snapshot(epoch_k, finalized_hash)`
        // would see an empty committee for any epoch > 0.
        if let Some(path) = &node.ext.dpos_staking_config {
            match fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(path) {
                Ok(parsed) => {
                    staking_address = parsed.staking_address;
                    chain_config_address = parsed.chain_config_address;
                    staking_reader_cfg = Some(parsed.clone());
                    // Both-or-neither: the committee-commit gate
                    // (evm.rs:870) needs BOTH addresses non-zero. A one-zero
                    // typo would silently downgrade the node to plain
                    // Ethereum and wedge at the first epoch boundary with no
                    // error — fail loud at load instead.
                    if staking_address.is_zero() != chain_config_address.is_zero() {
                        eprintln!(
                            "--dpos.staking-config partial config: staking_address \
                             ({staking_address}) and chain_config_address \
                             ({chain_config_address}) must be BOTH zero (non-DPoS) \
                             or BOTH non-zero (DPoS)"
                        );
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "failed parsing --dpos.staking-config at {}: {e}",
                        path.display()
                    );
                    std::process::exit(1);
                }
            }
        }
    }

    // Pre-spawn the DPoS thread before reth's runtime starts. The thread
    // blocks on a oneshot until the closure below forwards the reth
    // FullNode.
    let dpos_setup = dpos_config.map(|cfg| {
        let shutdown_token = CancellationToken::new();
        let spawn = spawn_dpos::<_, EthereumAddOns<_, _, _>>(cfg, shutdown_token.clone());
        (spawn, shutdown_token)
    });
    let cert_follow_setup = cert_follow_config.map(|cfg| {
        let shutdown_token = CancellationToken::new();
        let spawn = spawn_cert_follower::<_, EthereumAddOns<_, _, _>>(cfg, shutdown_token.clone());
        (spawn, shutdown_token)
    });

    // Split the consensus thread setup so cancel + join run AFTER
    // cli.run_with_components returns (Tempo pattern at bin/tempo/src/main.rs:734-742).
    // Joining inside the async closure risks deadlock when reth's engine shutdown
    // races with the consensus thread's in-flight beacon_engine_handle calls.
    // DPoS and cert-follow are mutually exclusive (clap `conflicts_with`), so at
    // most one setup is `Some`; both normalize to the same (handle_tx, dead_rx) +
    // (join, shutdown_token) shape the launch closure drives.
    let (consensus_thread_inner, consensus_thread_cleanup) = match (dpos_setup, cert_follow_setup) {
        (Some((spawn, token)), None) => (
            Some((spawn.handle_tx, spawn.dead_rx)),
            Some((spawn.join, token)),
        ),
        (None, Some((spawn, token))) => (
            Some((spawn.handle_tx, spawn.dead_rx)),
            Some((spawn.join, token)),
        ),
        (None, None) => (None, None),
        (Some(_), Some(_)) => {
            unreachable!("clap conflicts_with prevents --dpos together with --cert-follow")
        }
    };

    let components = move |spec: Arc<ChainSpec>| {
        (
            FluentEvmConfig::new(
                spec.clone(),
                fluentbase_node::evm::FluentEvmFactory::default(),
                staking_address,
                chain_config_address,
            ),
            Arc::new(FluentConsensus::new(spec)),
        )
    };

    let extra_data_registry_for_node = extra_data_registry.clone();
    let run_result = cli.run_with_components::<FluentNode>(components, async move |builder, _| {
        info!(target: "reth::cli", "Launching node");

        let components_builder = FluentNode::with_extra_data_registry(
            extra_data_registry_for_node,
            !staking_address.is_zero(),
        )
        .components_builder()
        .executor(FluentExecutorBuilder::new(
            staking_address,
            chain_config_address,
        ));
        let add_ons = EthereumAddOns::default();

        let handle: DebugNodeLauncherFuture<_, _, _> = builder
            .with_types::<FluentNode>()
            .with_components(components_builder)
            .with_add_ons(add_ons)
            .extend_rpc_modules(move |ctx| {
                // Register the `consensus` namespace (cert-follower server) when
                // this node serves the cert feed (DPoS enabled). Rides the
                // existing `--http`/`--ws` transports.
                if let Some(feed) = cert_rpc_feed {
                    ctx.modules
                        .merge_configured(ConsensusRpc::new(feed).into_rpc())?;
                }
                Ok(())
            })
            .launch_with_debug_capabilities();

        let handle = handle.await?;

        // Engine-shutdown watcher: independent task that triggers
        // `engine_shutdown.shutdown()` on SIGINT/SIGTERM so reth's
        // `persist_until_complete` flushes `tree_state.executed_blocks` to
        // MDBX before the runtime is torn down. `spawn_critical_with_graceful_shutdown_signal`
        // gives us a `GracefulShutdown` handle whose embedded
        // `GracefulShutdownGuard` increments the `graceful_tasks` counter
        // that `Runtime::do_graceful_shutdown` spins on — held in
        // `_shutdown_guard` for the duration of `done_rx.await` so the
        // runtime drain blocks on us. Without this, Docker SIGTERM races
        // reth's `run_until_ctrl_c` cancellation hierarchy and in-memory
        // blocks 10-11 are lost on a Tempo→DPoS swap.
        let engine_shutdown = handle.node.add_ons_handle.engine_shutdown.clone();
        handle
            .node
            .task_executor
            .spawn_critical_with_graceful_shutdown_signal(
                "fluent-engine-shutdown-watcher",
                move |graceful_shutdown| async move {
                    let _shutdown_guard = graceful_shutdown;

                    #[cfg(unix)]
                    let sigterm_fut = async {
                        match tokio::signal::unix::signal(
                            tokio::signal::unix::SignalKind::terminate(),
                        ) {
                            Ok(mut s) => {
                                let _ = s.recv().await;
                            }
                            Err(e) => {
                                error!(?e, "fluent-shutdown-watcher: SIGTERM stream setup failed");
                                std::future::pending::<()>().await;
                            }
                        }
                    };
                    #[cfg(not(unix))]
                    let sigterm_fut = std::future::pending::<()>();

                    let signal = tokio::select! {
                        res = tokio::signal::ctrl_c() => {
                            if let Err(e) = res {
                                error!(?e, "fluent-shutdown-watcher: ctrl_c await failed");
                            }
                            "SIGINT"
                        }
                        _ = sigterm_fut => "SIGTERM",
                    };
                    info!(
                        signal,
                        "fluent-shutdown-watcher: signal observed; triggering engine_shutdown"
                    );
                    flush_engine_in_memory(&engine_shutdown, "fluent-shutdown-watcher").await;
                },
            );

        if let Some(block_time) = block_producer {
            // Tempo→DPoS clean-halt: read the immutable, genesis-baked activation block
            // once and stop producing once the sequencer head reaches it. `0` ⇒ absolute
            // numbering (no migration) ⇒ no gate. Followers (--sequencer-url) relay the
            // sequencer and stop with it, so this single read gates the whole network.
            let activation_gate: Option<u64> = match &staking_reader_cfg {
                Some(cfg) if !cfg.chain_config_address.is_zero() => {
                    let reader = fluentbase_staking_reader::RethStakingStateReader::new(
                        handle.node.provider.clone(),
                        handle.node.evm_config.clone(),
                        cfg.clone(),
                    );
                    let best = handle.node.provider.best_block_number()?;
                    let best_hash = handle
                        .node
                        .provider
                        .sealed_header(best)?
                        .ok_or_eyre(
                            "sequencer: no sealed header at best block for activation read",
                        )?
                        .hash();
                    let act = reader.dpos_activation_block(best_hash)?;
                    (act > 0).then_some(act)
                }
                _ => None,
            };
            launch_consensus_validator(
                &handle,
                block_time,
                FluentPayloadAttributesBuilder {},
                activation_gate,
            )
            .await?;
        } else if let Some(consensus_url) = consensus_url {
            launch_consensus_node(&handle, consensus_url).await?;
        }

        if let Some((handle_tx, dead_rx)) = consensus_thread_inner {
            info!(target: "reth::cli", "Handing reth FullNode to consensus thread");
            if handle_tx.send(handle.node.clone()).is_err() {
                eyre::bail!("consensus thread exited before NodeHandle could be sent");
            }

            // Second `engine_shutdown` handle for the DPoS-death path: the
            // first clone is moved into the signal watcher above, so a DPoS
            // thread death (dead_rx) needs its own. Without flushing here,
            // reth tears down without persisting in-memory `executed_blocks`
            // to MDBX and a missing finalized block is fatal to DPoS restart.
            let engine_shutdown_dead = handle.node.add_ons_handle.engine_shutdown.clone();

            tokio::select! {
                _ = handle.node_exit_future => {
                    info!("reth execution node exited");
                }
                _ = dead_rx => {
                    info!("DPoS thread exited; flushing in-memory blocks");
                    flush_engine_in_memory(&engine_shutdown_dead, "dpos-death").await;
                }
            }
            Ok(())
        } else {
            handle.node_exit_future.await
        }
    });

    // Consensus-thread cleanup runs AFTER reth returns, regardless of
    // run_result — the thread (DPoS or cert-follow) always gets a chance to exit
    // cleanly.
    if let Some((join, shutdown_token)) = consensus_thread_cleanup {
        shutdown_token.cancel();
        match join.join() {
            Ok(Ok(())) => info!("consensus thread joined cleanly"),
            Ok(Err(e)) => error!(?e, "consensus thread exited with error"),
            Err(panic) => {
                error!("consensus thread panicked");
                std::panic::resume_unwind(panic);
            }
        }
    }

    if let Err(err) = run_result {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
