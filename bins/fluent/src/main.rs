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
use reth_node_ethereum::EthereumAddOns;
use std::{borrow::Cow, sync::Arc, time::Duration};
use tracing::info;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

#[cfg(all(feature = "jemalloc-prof", unix))]
#[unsafe(export_name = "_rjem_malloc_conf")]
static MALLOC_CONF: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

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

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    // Initialize default download URLs for snapshots
    init_downloads_defaults();

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

        let handle: DebugNodeLauncherFuture<_, _, _> = builder
            .with_types::<FluentNode>()
            .with_components(components_builder)
            .with_add_ons(add_ons)
            .launch_with_debug_capabilities();

        let handle = handle.await?;

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
