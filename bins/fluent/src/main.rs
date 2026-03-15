#![allow(missing_docs, dead_code)]

use clap::{Args, Parser};
use fluent::{
    chainspec::FluentChainSpecParser,
    consensus::{launch_consensus_node, launch_consensus_validator},
    evm::{FluentEvmConfig, FluentExecutorBuilder, FluentNode},
    payload::FluentPayloadAttributesBuilder,
    trusted_peers::{resolve_default_consensus_url, resolve_default_trusted_peers},
};
use humantime::parse_duration;
use reth_chainspec::ChainSpec;
use reth_ethereum_cli::{Cli, Commands};
use reth_ethereum_consensus::EthBeaconConsensus;
use reth_node_builder::{DebugNodeLauncherFuture, Node};
use reth_node_ethereum::EthereumAddOns;
use std::{sync::Arc, time::Duration};
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
}

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    let mut consensus_url: Option<String> = None;
    // let mut block_finalizer_sidecar_url: Option<String> = None;
    let mut block_producer: Option<Duration> = None;

    let mut cli = Cli::<FluentChainSpecParser, FluentNodeArgs>::parse();
    if let Commands::Node(node) = &mut cli.command {
        let new_trusted_peers = resolve_default_trusted_peers(node.chain.chain);
        node.network.trusted_peers.extend(new_trusted_peers);
        // If consensus URL is not specified, resolve default
        if let Some(debug_consensus_url) = &node.debug.rpc_consensus_url {
            consensus_url = Some(debug_consensus_url.clone());
        } else {
            consensus_url = resolve_default_consensus_url(node.chain.chain);
        }
        if node.ext.validator {
            block_producer = Some(node.ext.validator_block_time);
        }
    }

    let components = |spec: Arc<ChainSpec>| {
        (
            FluentEvmConfig::new_with_default_factory(spec.clone()),
            Arc::new(EthBeaconConsensus::new(spec)),
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
