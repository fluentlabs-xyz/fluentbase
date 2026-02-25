#![allow(missing_docs, dead_code)]

use clap::Parser;
use eyre::{eyre, Result};
use fluent::{
    chainspec::{FLUENT_DEVNET, FLUENT_TESTNET},
    evm::FluentEvmFactory,
};
use reth_chainspec::ChainSpec;
use reth_db::{
    mdbx::{open_db, DatabaseArguments},
    DatabaseEnv,
};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{execute::Executor, ConfigureEvm};
use reth_network::types::BlockHashOrNumber;
use reth_node_api::NodeTypesWithDBAdapter;
use reth_node_core::primitives::AlloyBlockHeader;
use reth_node_ethereum::{EthEvmConfig, EthereumNode};
use reth_primitives::{Block, Header, RecoveredBlock};
use reth_provider::{
    providers::{BlockchainProvider, RocksDBProvider, StaticFileProvider},
    BlockReader, BlockReaderIdExt, HeaderProvider, ProviderFactory, StateProviderFactory,
    StaticFileProviderBuilder, TransactionVariant,
};
use reth_revm::database::StateProviderDatabase;
use std::{fmt, path::PathBuf, sync::Arc};

#[derive(Parser, Debug)]
#[command(name = "reth-transition-test")]
struct Args {
    /// Path to reth datadir (folder that contains the DB)
    #[arg(long, default_value = "bins/chain-transition-verifier/datadir")]
    datadir: PathBuf,

    /// Chain: fluent-devnet|fluent-testnet|fluent-mainnet (extend as needed)
    #[arg(long, default_value = "fluent-devnet")]
    chain: String,

    /// First block to test (inclusive)
    #[arg(long, default_value = "0")]
    from: u64,

    /// Print every block ok line
    #[arg(long, default_value_t = true)]
    verbose: bool,
}

type Node = NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>;

// #[cfg(feature = "fluent-testnet")]
// tables! {
//     /// Stores generic node metadata as key-value pairs.
//     /// Can store feature flags, configuration markers, and other node-specific data.
//     table Metadata {
//         type Key = String;
//         type Value = Vec<u8>;
//     }
// }

fn main() -> Result<()> {
    let args = Args::parse();

    let db_path = args.datadir.join("db");
    let static_file_path = args.datadir.join("static_files");
    let rocks_db_path = args.datadir.join("rocksdb");
    let db = open_db(db_path, DatabaseArguments::default())
        .map_err(|e| eyre!("failed to open database: {e}"))?;
    // #[cfg(feature = "fluent-testnet")]
    // db.create_and_track_tables_for::<Tables>()?;
    let db = Arc::new(db);

    let chain_spec: Arc<ChainSpec> = match args.chain.as_str() {
        "fluent-devnet" => FLUENT_DEVNET.clone(),
        "fluent-testnet" => FLUENT_TESTNET.clone(),
        other => {
            return Err(eyre!(
                "unsupported --chain {other} (patch: load your custom ChainSpec)"
            ))
        }
    };

    let static_file_provider: StaticFileProvider<EthPrimitives> =
        StaticFileProviderBuilder::read_write(static_file_path).build()?;
    let rocksdb_provider = RocksDBProvider::new(rocks_db_path.as_path())?;

    let provider_factory: ProviderFactory<Node> = ProviderFactory::new(
        db,
        chain_spec.clone(),
        static_file_provider,
        rocksdb_provider,
        Default::default(),
    )?;
    let blockchain = BlockchainProvider::new(provider_factory)
        .map_err(|e| eyre!("failed to init BlockchainProvider: {e}"))?;
    let evm_config = EthEvmConfig::<ChainSpec, FluentEvmFactory>::new_with_evm_factory(
        chain_spec.clone(),
        FluentEvmFactory::default(),
    );

    let latest_header = blockchain
        .latest_header()?
        .ok_or_else(|| eyre!("no genesis header found"))?;
    let latest_block_number = latest_header.number;
    if args.verbose {
        eprintln!("latest block number: {}", latest_block_number);
    }

    for n in args.from..=latest_block_number {
        if args.verbose && n % 10_000 == 0 {
            eprintln!(
                "checking block: {n} ({:.2}%)",
                n as f64 / latest_block_number as f64 * 100.0
            );
        }

        let header: Header = blockchain
            .header_by_number(n)?
            .ok_or_else(|| eyre!("missing header for block {n}"))?;

        let recovered: RecoveredBlock<Block> = blockchain
            .sealed_block_with_senders(BlockHashOrNumber::Number(n), TransactionVariant::NoHash)?
            .ok_or_else(|| eyre!("missing block body for block {n}"))?;

        if args.verbose && recovered.transaction_count() > 0 {
            eprintln!(
                " found block {n} with {} transactions",
                recovered.transaction_count()
            );
        }

        let parent = n.saturating_sub(1);
        let state_provider = blockchain
            .state_by_block_number_or_tag(parent.into())
            .map_err(|e| eyre!("failed to get state provider at block {parent}: {e}"))?;
        let db = StateProviderDatabase::new(state_provider);

        let executor = evm_config.executor(db);
        let result = executor
            .execute(&recovered)
            .map_err(|e| eyre!("❌ transition mismatch / execution error at block {n}: {e}"))?;
        if result.gas_used != recovered.gas_used {
            return Err(eyre!(
                "❌ block {n} gas used mismatch: {} != {} (should be)",
                result.gas_used,
                recovered.gas_used
            ));
        }

        if args.verbose {
            eprintln!("✅ block {n} ok (state_root {:?})", header.state_root());
        }
    }

    Ok(())
}
