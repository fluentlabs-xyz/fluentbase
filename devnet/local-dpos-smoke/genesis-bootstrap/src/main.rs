use clap::Parser;
use eyre::WrapErr;
use std::net::IpAddr;
use std::path::PathBuf;

use fluentbase_genesis_bootstrap::{artifacts, bootstrap, genesis, keys, output};

#[derive(Parser, Debug)]
#[command(
    about = "Generate a deterministic local-smoke DPoS genesis + per-validator key set"
)]
struct Args {
    #[arg(long, default_value_t = 4)]
    peers: u32,

    #[arg(long, default_value_t = 1)]
    bootstrappers: u32,

    #[arg(long, default_value = "/runtime")]
    output: PathBuf,

    #[arg(long, env = "CONTRACTS_DIR", default_value = "/contracts")]
    contracts_dir: PathBuf,

    /// Default mnemonic is the foundry/hardhat-canonical test mnemonic
    /// (`test test ... junk`). Identifies this chain as a developer
    /// smoke; do NOT reuse for anything that touches real value.
    #[arg(
        long,
        env = "FLUENT_DPOS_MNEMONIC",
        default_value = "test test test test test test test test test test test junk"
    )]
    mnemonic: String,

    #[arg(long, default_value_t = 2026)]
    chain_id: u64,

    /// Comma-separated list of pinned validator IPs (one per peer).
    /// docker-compose `networks: fluent-net` ipam block pins these,
    /// genesis-bootstrap writes them into `peers.json` because the
    /// fluent CLI / bootstrappers JSON deserialise socket as
    /// `SocketAddr` (IP literal only, not hostname).
    #[arg(long, value_delimiter = ',')]
    validator_ips: Vec<IpAddr>,
}

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    eyre::ensure!(
        ![1337u64, 20993, 20994, 25363].contains(&args.chain_id),
        "chain id {} collides with public fluent network",
        args.chain_id
    );
    eyre::ensure!(
        args.peers >= 1 && args.bootstrappers >= 1 && args.bootstrappers <= args.peers,
        "invalid --peers / --bootstrappers"
    );
    eyre::ensure!(
        args.validator_ips.len() == args.peers as usize,
        "--validator-ips count ({}) must match --peers ({})",
        args.validator_ips.len(),
        args.peers
    );

    std::fs::create_dir_all(&args.output).wrap_err("create output dir")?;

    let key_set = keys::derive(&args.mnemonic, args.peers, args.chain_id)?;
    tracing::info!(
        peers = args.peers,
        chain_id = args.chain_id,
        "keys derived deterministically from mnemonic"
    );

    let artefacts = artifacts::load(&args.contracts_dir)?;
    tracing::info!(
        contracts_dir = %args.contracts_dir.display(),
        "vendored forge artefacts loaded"
    );

    let predeploy_state = bootstrap::run(&key_set, &artefacts, args.chain_id)?;
    let genesis = genesis::assemble(args.chain_id, &key_set, predeploy_state)?;

    output::write(
        &args.output,
        &genesis,
        &key_set,
        args.bootstrappers as usize,
        &args.validator_ips,
    )?;
    tracing::info!(out = %args.output.display(), "bootstrap complete");
    Ok(())
}
