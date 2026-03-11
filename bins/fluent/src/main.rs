#![allow(missing_docs, dead_code)]

use clap::Parser;
use fluent::{
    chainspec::FluentChainSpecParser,
    evm::FluentExecutorBuilder,
    payload::FluentPayloadAttributesBuilder,
    trusted_peers::{resolve_default_consensus_url, resolve_default_trusted_peers},
};
use reth_ethereum_cli::{Cli, Commands};
use reth_node_builder::{Node, NodeHandle};
use reth_node_ethereum::{EthereumAddOns, EthereumNode};
use tracing::info;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

#[cfg(all(feature = "jemalloc-prof", unix))]
#[unsafe(export_name = "_rjem_malloc_conf")]
static MALLOC_CONF: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    let mut cli = Cli::<FluentChainSpecParser>::parse();
    if let Commands::Node(node) = &mut cli.command {
        let new_trusted_peers = resolve_default_trusted_peers(node.chain.chain);
        node.network.trusted_peers.extend(new_trusted_peers);
        // If consensus URL is not specified, resolve default
        if node.debug.rpc_consensus_url.is_none() {
            node.debug.rpc_consensus_url = resolve_default_consensus_url(node.chain.chain);
        }
    }

    if let Err(err) = cli.run(async move |builder, _| {
        info!(target: "reth::cli", "Launching node");

        let components_builder = EthereumNode::default()
            .components_builder()
            .executor(FluentExecutorBuilder::default());
        let add_ons = EthereumAddOns::default();

        let NodeHandle {
            node: _node,
            node_exit_future,
        } = builder
            .with_types::<EthereumNode>()
            .with_components(components_builder)
            .with_add_ons(add_ons)
            .launch_with_debug_capabilities()
            .with_payload_attributes_builder(FluentPayloadAttributesBuilder {})
            .await?;

        node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
