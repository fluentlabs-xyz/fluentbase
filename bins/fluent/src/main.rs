#![allow(missing_docs, dead_code)]

mod node_modes;

use alloy_primitives::Bytes;
use alloy_rpc_types_engine::PayloadId;
use clap::{Args, Parser};
use dashmap::DashMap;
use fluentbase_node::{
    cert_follow::spawn_cert_follower,
    chainspec::FluentChainSpecParser,
    consensus::FluentConsensus,
    consensus_rpc::{ConsensusApiServer, ConsensusRpc},
    dpos::{spawn_dpos, DposArgs},
    evm::{FluentEvmConfig, FluentExecutorBuilder, FluentNode},
    launcher::{launch_consensus_node, launch_consensus_validator, ActivationProbe},
    payload::FluentPayloadAttributesBuilder,
    trusted_peers::resolve_default_trusted_peers,
};
use humantime::parse_duration;
use reth_chainspec::ChainSpec;
use reth_cli_commands::download::DownloadDefaults;
use reth_ethereum_cli::{Cli, Commands};
use reth_node_builder::{DebugNodeLauncherFuture, Node};
use reth_node_core::version::{default_reth_version_metadata, try_init_version_metadata};
use reth_node_ethereum::EthereumAddOns;
use reth_storage_api::{BlockNumReader, HeaderProvider};
use std::{borrow::Cow, sync::Arc, time::Duration};
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
    #[arg(
        long = "validator",
        default_value_t = false,
        conflicts_with = "sequencer_url"
    )]
    pub validator: bool,

    #[arg(
        long = "validator.block-time",
        value_parser = parse_duration,
        default_value = "1s",
    )]
    pub validator_block_time: Duration,

    /// Repeatable: extra occurrences form the cert-follow failover list
    /// (rotated on connect failure/disconnect). Trust-follow uses the first.
    #[arg(long = "sequencer-url", action = clap::ArgAction::Append)]
    pub sequencer_url: Vec<String>,

    /// Run as a DPoS validator (BFT consensus + p2p + finality-gated peer set).
    /// Mutually exclusive with --validator and --sequencer-url.
    #[arg(
        long = "dpos",
        default_value_t = false,
        conflicts_with_all = &["validator", "sequencer_url"],
        // Require exactly one BLS key flag when --dpos: the `bls` ArgGroup
        // (both flags, `conflicts_with` ⇒ at-most-one) plus this `requires_if`
        // (⇒ at-least-one) = exactly-one, caught at PARSE time instead of after
        // reth has fully launched (the exit-0-after-launch class; audit P2-18).
        requires_if("true", "bls"),
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

    /// L1 RPC for the cert-follower's Rollup trust-root checkpoint
    /// (read at the `finalized` tag). Absent = devnet fallback (the upstream
    /// head stays the only trust input).
    #[arg(
        long = "cert-follow.l1-rpc-url",
        requires = "cert_follow_l1_rollup_address"
    )]
    pub cert_follow_l1_rpc_url: Option<String>,

    /// Rollup contract address on L1 (pairs with --cert-follow.l1-rpc-url).
    #[arg(
        long = "cert-follow.l1-rollup-address",
        requires = "cert_follow_l1_rpc_url"
    )]
    pub cert_follow_l1_rollup_address: Option<alloy_primitives::Address>,

    /// DPoS validator configuration (`--dpos.*`): keys, paths, ports. Flattened
    /// so the long flag list lives next to `DposConfig` in `fluentbase-node`.
    #[command(flatten)]
    pub dpos_cfg: DposArgs,
}

fn init_downloads_defaults() {
    let download_defaults = DownloadDefaults {
        available_snapshots: vec![Cow::Borrowed(
            "https://cdn.fluent.xyz/snapshots/20994/fluent-testnet-22459308.tar.gz",
        )],
        default_base_url: Cow::Borrowed("https://cdn.fluent.xyz/snapshots"),
        default_chain_aware_base_url: Some(Cow::Borrowed("https://cdn.fluent.xyz/snapshots")),
        snapshot_api_url: Cow::Borrowed("https://api.fluent.xyz/snapshots"),
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

    // Single shared `Arc<DashMap>` instance
    // between `FluentNode`'s payload builder (reader) and the DPoS thread's
    // `OuterBuilder.extra_data_registry` (writer). Empty in non-DPoS modes
    // — the executor's `processBitmap` system call no-ops on
    // `committeeSize == 0` decoded from an empty extra_data.
    let extra_data_registry: Arc<DashMap<PayloadId, Bytes>> = Arc::new(DashMap::new());

    let mut cli = Cli::<FluentChainSpecParser, FluentNodeArgs>::parse();

    // Adjust several params for node execution + resolve the fluent node modes.
    let modes = if let Commands::Node(node) = &mut cli.command {
        // Merge default public trusted peers (reth network config).
        let new_trusted_peers = resolve_default_trusted_peers(node.chain.chain);
        node.network.trusted_peers.extend(new_trusted_peers);

        node_modes::resolve_node_modes(
            &node.ext,
            node.chain.chain,
            node.debug.rpc_consensus_url.as_deref(),
        )
    } else {
        node_modes::resolve_non_node_modes()
    };
    let node_modes::ResolvedModes {
        consensus_url,
        block_producer,
        dpos_config,
        cert_follow_config,
        cert_rpc_feed,
        staking_address,
        chain_config_address,
        liveness_slashing_address,
        staking_reader_cfg,
    } = modes;

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
                liveness_slashing_address,
            ),
            Arc::new(FluentConsensus::new(spec)),
        )
    };

    let extra_data_registry_for_node = extra_data_registry.clone();

    // RocksDB close-order guard. The consensus thread (slasher `PoolTxSink`, and
    // any other DPoS/cert-follow consumer) clones reth's provider, which owns a
    // RocksDB instance. If such a clone is the LAST reference it closes RocksDB
    // from the commonware runtime-teardown path, where RocksDB's process-global
    // `PeriodicTaskScheduler::timer_mutex` is torn down concurrently -> a glibc
    // "pthread lock: Invalid argument" abort (intermittent shutdown SIGABRT,
    // exit 134/139 AFTER a successful flush — data is safe, but the process
    // crashes on the way out). Send a provider clone out to `main` and hold it
    // until AFTER the consensus thread is joined, so RocksDB's final close runs
    // exactly once, on the main thread, in a clean context after every other ref
    // is gone.
    let (rocksdb_keepalive_tx, rocksdb_keepalive_rx) =
        std::sync::mpsc::sync_channel::<Box<dyn std::any::Any + Send>>(1);

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
            liveness_slashing_address,
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

        // Defer RocksDB's final close to `main` (see rocksdb_keepalive_tx): hand a
        // provider clone out so the consensus thread's clone is never the last ref.
        let _ = rocksdb_keepalive_tx.send(Box::new(handle.node.provider.clone()));

        // Tempo→DPoS activation probe, shared by the producer's clean-halt
        // gate and the trust-follower's two-tier finality mirror. Re-read per
        // tick/block (NOT once at launch): a node started before governance
        // schedules activation must still gate / mirror without a restart,
        // and a pending activation may be re-scheduled. Pre-deploy (codeless
        // ChainConfig) and unscheduled (0) both map to None; consumers latch
        // the last Some, so every failure path below must be observable
        // (warn) — a silent None is indistinguishable from "not scheduled
        // yet" and would hide a degraded provider from operators.
        let activation_probe: Option<ActivationProbe> = match &staking_reader_cfg {
            Some(cfg) if !cfg.chain_config_address.is_zero() => {
                let reader = fluentbase_staking_reader::RethStakingStateReader::new(
                    handle.node.provider.clone(),
                    handle.node.evm_config.clone(),
                    cfg.clone(),
                );
                let provider = handle.node.provider.clone();
                Some(Arc::new(move || {
                    let best_hash = match provider
                        .best_block_number()
                        .ok()
                        .and_then(|n| provider.sealed_header(n).ok().flatten())
                        .map(|h| h.hash())
                    {
                        Some(hash) => hash,
                        None => {
                            tracing::warn!(
                                target: "reth::cli",
                                "DPoS activation probe: best-header read failed; \
                                 keeping last known value"
                            );
                            return None;
                        }
                    };
                    match reader.scheduled_dpos_activation(best_hash) {
                        Ok(act) => act,
                        Err(e) => {
                            tracing::warn!(
                                target: "reth::cli",
                                error = %e,
                                "DPoS activation probe failed; keeping last known value"
                            );
                            None
                        }
                    }
                }) as ActivationProbe)
            }
            _ => None,
        };
        if let Some(block_time) = block_producer {
            launch_consensus_validator(
                &handle,
                block_time,
                FluentPayloadAttributesBuilder {},
                activation_probe,
            )
            .await?;
        } else if let Some(consensus_url) = consensus_url {
            launch_consensus_node(&handle, consensus_url, activation_probe).await?;
        }

        if let Some((handle_tx, dead_rx)) = consensus_thread_inner {
            info!(target: "reth::cli", "Handing reth FullNode to consensus thread");
            if handle_tx.send(handle.node.clone()).is_err() {
                eyre::bail!("consensus thread exited before NodeHandle could be sent");
            }

            tokio::select! {
                _ = handle.node_exit_future => {
                    info!("reth execution node exited");
                }
                _ = dead_rx => {
                    // No flush: reth persists its in-memory tail on its own graceful
                    // exit, and any unpersisted tail is rebuilt on the next cold-start
                    // via `recover_finalized_tail_into_reth` (marshal durable archive).
                    info!("DPoS thread exited; reth persists natively, lost tail \
                           recovers on next cold-start");
                }
            }
            Ok(())
        } else {
            handle.node_exit_future.await
        }
    });

    // Hold reth's provider (the RocksDB owner) alive across the consensus-thread
    // join so the slasher's clone can never be the last reference (see
    // rocksdb_keepalive_tx). `None` if the node closure errored before sending.
    let rocksdb_keepalive = rocksdb_keepalive_rx.recv().ok();

    // Consensus-thread cleanup runs AFTER reth returns, regardless of
    // run_result — the thread (DPoS or cert-follow) always gets a chance to exit
    // cleanly.
    let mut consensus_failed = false;
    if let Some((join, shutdown_token)) = consensus_thread_cleanup {
        shutdown_token.cancel();
        match join.join() {
            Ok(Ok(())) => info!("consensus thread joined cleanly"),
            Ok(Err(e)) => {
                // A dead validator/follower (boot misconfig, wrong keystore
                // password, or a mid-run consensus fault that cancelled the shared
                // token via the 3-strike escalation) must exit NON-ZERO so systemd
                // `Restart=on-failure` and exit-code alerting fire — otherwise the
                // node silently stops attesting (audit P2-4 / P2-17).
                error!(?e, "consensus thread exited with error");
                consensus_failed = true;
            }
            Err(panic) => {
                error!("consensus thread panicked");
                std::panic::resume_unwind(panic);
            }
        }
    }

    // Every other reth/consensus reference is now gone; drop the keepalive so
    // RocksDB's final close runs here, on the main thread, in a clean context
    // (not from the consensus runtime teardown that triggered the SIGABRT).
    drop(rocksdb_keepalive);

    // Print reth's error FIRST (it is often the root cause of a correlated crash),
    // then exit non-zero if either reth or the consensus thread failed — don't let
    // the consensus-thread failure shadow reth's reason from stderr.
    if let Err(err) = &run_result {
        eprintln!("Error: {err:?}");
    }
    if consensus_failed || run_result.is_err() {
        std::process::exit(1);
    }
}
