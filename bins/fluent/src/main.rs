#![allow(missing_docs, dead_code)]

use clap::Parser;
use fluent::{chainspec::FluentChainSpecParser, evm::FluentExecutorBuilder};
use reth_ethereum_cli::Cli;
use reth_node_builder::Node;
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

    if let Err(err) = Cli::<FluentChainSpecParser>::parse().run(async move |builder, _| {
        info!(target: "reth::cli", "Launching node");

        let components_builder = EthereumNode::default()
            .components_builder()
            .executor(FluentExecutorBuilder::default());
        let add_ons = EthereumAddOns::default();

        let handle = builder
            .with_types::<EthereumNode>()
            .with_components(components_builder)
            .with_add_ons(add_ons)
            .launch_with_debug_capabilities()
            .await?;
        handle.wait_for_node_exit().await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
