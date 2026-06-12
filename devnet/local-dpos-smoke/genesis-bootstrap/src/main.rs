use clap::{Parser, Subcommand};
use commonware_codec::Encode as _;
use commonware_cryptography::Signer as _;
use eyre::WrapErr;
use serde::Serialize;
use std::net::IpAddr;
use std::path::PathBuf;

use fluentbase_genesis_bootstrap::{artifacts, bootstrap, genesis, keys, output, pop};

const DEFAULT_MNEMONIC: &str = "test test test test test test test test test test test junk";

#[derive(Parser, Debug)]
#[command(about = "Generate a deterministic local-smoke DPoS genesis + per-validator key set")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Deploy the staking cluster into genesis (genesis-baked smoke).
    Full(GenesisArgs),
    /// Plain chain: keys + funding + genesis, NO staking cluster. The staking
    /// cluster is deployed at runtime via `forge` (production-path smoke), so
    /// `staking-reader.json` is written later by the driver, not here.
    Bare(GenesisArgs),
    /// Emit one validator's consensus-key material (BLS pubkey/PoP, peer pubkey,
    /// l2 owner address + key) as JSON, for host-side `cast setConsensusKeys`.
    ConsensusKeys(ConsensusKeysArgs),
}

#[derive(clap::Args, Debug)]
struct GenesisArgs {
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
    #[arg(long, env = "FLUENT_DPOS_MNEMONIC", default_value = DEFAULT_MNEMONIC)]
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

    /// Bare mode only: also write `staking-reader.json` predicting the
    /// staking cluster a driver will forge-deploy at runtime. Three CREATE
    /// nonces of the owner-0 deployer — the staking, chain_config, and
    /// liveness_slashing proxies — resolved in-process via
    /// `Address::create`. The production-path driver fail-loud asserts the
    /// post-deploy manifest equals this file.
    #[arg(long, value_delimiter = ',')]
    staking_reader_create_nonces: Option<Vec<u64>>,
}

#[derive(clap::Args, Debug)]
struct ConsensusKeysArgs {
    /// Validator index whose consensus-key material to emit.
    #[arg(long)]
    idx: u32,

    #[arg(long, default_value_t = 2026)]
    chain_id: u64,

    #[arg(long, env = "FLUENT_DPOS_MNEMONIC", default_value = DEFAULT_MNEMONIC)]
    mnemonic: String,

    /// Total validator pool size — must match the genesis-bootstrap `--peers`
    /// so the per-index key derivation is identical.
    #[arg(long, default_value_t = 6)]
    peers: u32,
}

/// JSON fields are the four `setConsensusKeys(address,bytes,bytes,bytes32)` args
/// plus the owner key the host signs the call with. camelCase mirrors the
/// Solidity ABI names so the bash driver can `jq` them straight into `cast`.
#[derive(Serialize)]
struct ConsensusKeyOutput {
    #[serde(rename = "validatorAddress")]
    validator_address: String,
    #[serde(rename = "blsPubkeyUncompressed")]
    bls_pubkey_uncompressed: String,
    #[serde(rename = "blsPoPUncompressed")]
    bls_pop_uncompressed: String,
    #[serde(rename = "peerPubkey")]
    peer_pubkey: String,
    #[serde(rename = "ownerKey")]
    owner_key: String,
}

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    match Cli::parse().cmd {
        Cmd::Full(args) => run_genesis(args, false),
        Cmd::Bare(args) => run_genesis(args, true),
        Cmd::ConsensusKeys(args) => run_consensus_keys(args),
    }
}

fn run_genesis(args: GenesisArgs, bare: bool) -> eyre::Result<()> {
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
    // Bare-only flag: GenesisArgs is shared with `full`, where a silently
    // ignored prediction would mask a misconfiguration (genesis bakes the
    // cluster at fixed predeploy slots — there is nothing to predict).
    eyre::ensure!(
        bare || args.staking_reader_create_nonces.is_none(),
        "--staking-reader-create-nonces is bare-mode-only (full bakes the \
         staking cluster into genesis at fixed predeploy slots)"
    );
    if let Some(nonces) = &args.staking_reader_create_nonces {
        eyre::ensure!(
            nonces.len() == 3,
            "--staking-reader-create-nonces needs exactly 3 values \
             (staking,chain_config,liveness_slashing), got {}",
            nonces.len()
        );
    }

    std::fs::create_dir_all(&args.output).wrap_err("create output dir")?;

    let key_set = keys::derive(&args.mnemonic, args.peers, args.chain_id)?;
    tracing::info!(
        peers = args.peers,
        chain_id = args.chain_id,
        bare,
        "keys derived deterministically from mnemonic"
    );

    // Bare mode skips the in-process staking deploy entirely: the cluster is
    // deployed at runtime via `forge`, so genesis carries an empty predeploy set
    // (`artifacts::load` is only consumed by `bootstrap::run`).
    let predeploy_state = if bare {
        bootstrap::PredeployState::default()
    } else {
        let artefacts = artifacts::load(&args.contracts_dir)?;
        tracing::info!(
            contracts_dir = %args.contracts_dir.display(),
            "vendored forge artefacts loaded"
        );
        bootstrap::run(&key_set, &artefacts, args.chain_id)?
    };

    let genesis = genesis::assemble(args.chain_id, &key_set, predeploy_state)?;

    output::write(
        &args.output,
        &genesis,
        &key_set,
        args.bootstrappers as usize,
        &args.validator_ips,
        bare,
        args.staking_reader_create_nonces.as_deref(),
    )?;
    tracing::info!(out = %args.output.display(), "bootstrap complete");
    Ok(())
}

fn run_consensus_keys(args: ConsensusKeysArgs) -> eyre::Result<()> {
    eyre::ensure!(
        args.idx < args.peers,
        "--idx {} out of range for --peers {}",
        args.idx,
        args.peers
    );

    let key_set = keys::derive(&args.mnemonic, args.peers, args.chain_id)?;
    let v = &key_set.validators[args.idx as usize];
    let pop_art = pop::produce(&v.bls, args.chain_id)?;
    let peer_pubkey = v.peer.public_key().encode();

    let out = ConsensusKeyOutput {
        validator_address: format!("{:#x}", v.l2_signer.address()),
        bls_pubkey_uncompressed: format!("0x{}", hex::encode(pop_art.bls_pubkey_uncompressed)),
        bls_pop_uncompressed: format!("0x{}", hex::encode(pop_art.bls_pop_uncompressed)),
        peer_pubkey: format!("0x{}", hex::encode(peer_pubkey.as_ref())),
        owner_key: format!("0x{}", hex::encode(v.l2_signer.to_bytes())),
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
