#![allow(missing_docs, dead_code)]

use clap::{Args, Parser};
use fluentbase_node::{
    chainspec::FluentChainSpecParser,
    consensus::FluentConsensus,
    evm::{FluentEvmConfig, FluentExecutorBuilder, FluentNode},
    launcher::{launch_consensus_node, launch_consensus_validator},
    payload::FluentPayloadAttributesBuilder,
    trusted_peers::{resolve_default_consensus_url, resolve_default_trusted_peers},
};
use humantime::parse_duration;
use reth_chainspec::ChainSpec;
use reth_cli_commands::download::DownloadDefaults;
use reth_ethereum_cli::{Cli, Commands};
use reth_node_builder::{DebugNodeLauncherFuture, Node};
use reth_node_core::version::{default_reth_version_metadata, try_init_version_metadata};
use reth_node_ethereum::EthereumAddOns;
use std::{borrow::Cow, sync::Arc, time::Duration};
use tracing::info;

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

    // SAFETY: single-threaded at this point.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    // Initialize default download URLs for snapshots
    init_downloads_defaults();

    // Override default reth version metadata with fluentbase-specific build metadata.
    init_fluent_version_metadata();

    let mut consensus_url: Option<String> = None;
    let mut block_producer: Option<Duration> = None;

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
    }

    let components = |spec: Arc<ChainSpec>| {
        (
            FluentEvmConfig::new_with_default_factory(spec.clone()),
            Arc::new(FluentConsensus::new(spec)),
        )
    };

    if let Err(err) = cli.run_with_components::<FluentNode>(components, async move |builder, _| {
        info!(target: "reth::cli", "Launching node");

        let components_builder = FluentNode::default()
            .components_builder()
            .executor(FluentExecutorBuilder::default());
        let add_ons = EthereumAddOns::default();

        #[cfg(feature = "exex")]
        let handle = {
            use witness_courier::hub::WitnessHub;
            use tracing::{error};

            let hub = Arc::new(WitnessHub::new());

            let addr: std::net::SocketAddr = std::env::var("FLUENT_WITNESS_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:10000".into())
                .parse()
                .expect("invalid FLUENT_WITNESS_ADDR");

            let svc = witness_courier::server::create_service(Arc::clone(&hub));
            let server = tonic::transport::Server::builder()
                .add_service(svc)
                .serve(addr);

            let hub_exex = Arc::clone(&hub);

            let launch: DebugNodeLauncherFuture<_, _, _> = builder
                .with_types::<FluentNode>()
                .with_components(components_builder)
                .with_add_ons(add_ons)
                .install_exex("fluent-proving", move |ctx| async move {
                    Ok(fluent_exex::exex_main_loop(ctx, None, hub_exex))
                })
                .launch_with_debug_capabilities();

            let handle = launch.await?;

            handle.node.task_executor.spawn_task(Box::pin(async move {
                info!(%addr, "gRPC witness server starting");
                if let Err(e) = server.await {
                    error!(err = %e, "gRPC witness server failed");
                }
            }));

            handle
        };

        #[cfg(not(feature = "exex"))]
        let handle = {
            let launch: DebugNodeLauncherFuture<_, _, _> = builder
                .with_types::<FluentNode>()
                .with_components(components_builder)
                .with_add_ons(add_ons)
                .launch_with_debug_capabilities();

            launch.await?
        };

        if let Some(block_time) = block_producer {
            launch_consensus_validator(&handle, block_time, FluentPayloadAttributesBuilder {})
                .await?;
        } else if let Some(consensus_url) = consensus_url {
            launch_consensus_node(&handle, consensus_url).await?;
        }

        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}