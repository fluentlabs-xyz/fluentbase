use clap::{Parser, Subcommand};
use commonware_codec::Encode as _;
use commonware_cryptography::Signer as _;
use eyre::WrapErr;
use serde::Serialize;
use std::net::IpAddr;
use std::path::PathBuf;

use fluentbase_genesis_bootstrap::{
    artifacts, bootstrap, genesis, keys, output, output::PeerHostMode, pop,
};

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
    /// Plain chain: keys + funding + genesis + the Governor, NO staking module. The
    /// module is delivered to the running chain through the runtime-upgrade precompile
    /// (production-path smoke); the Governor has to be here because its address is
    /// compiled into that module and nothing can place code at a fixed address after
    /// genesis. Needs `--contracts-dir` for `FluentGovernance.json`.
    Bare(GenesisArgs),
    /// Emit one validator's consensus-key material (BLS pubkey/PoP, peer pubkey,
    /// l2 owner address + key) as JSON, for host-side `cast setConsensusKeys`.
    ConsensusKeys(ConsensusKeysArgs),
    /// Materialize ONE validator index's PRIVATE key files (bls/peer/slasher +
    /// owner key) onto disk and (re)write `addresses.json[0..=idx]`, WITHOUT
    /// re-running genesis. The soak's mint-on-demand uses this to create a fresh
    /// idx >= POOL at runtime (the only path that writes a single idx's secrets).
    WriteKeys(WriteKeysArgs),
}

/// `DPOS_ACTIVATION_BLOCK` from the environment, with EMPTY treated as UNSET.
///
/// The compose files forward `${DPOS_ACTIVATION_BLOCK:-}`, so the variable is present and
/// empty whenever the host has not tuned it — and the value they mean by that is "derive
/// the default from the interval", which is what `None` expresses here. A malformed
/// non-empty value is a real mistake and fails loudly rather than falling back.
fn env_activation_block() -> Option<u64> {
    let raw = std::env::var("DPOS_ACTIVATION_BLOCK").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(
        raw.parse()
            .unwrap_or_else(|e| panic!("DPOS_ACTIVATION_BLOCK={raw:?} is not a u64: {e}")),
    )
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
    /// docker-compose `networks: fluent-net` ipam block pins these;
    /// genesis-bootstrap writes them into `peers.json` in the default
    /// `--peer-host-mode=ip` (the `socket` field accepts either an IP literal
    /// or a DNS hostname, so `--peer-host-mode=dns` emits `validator-N:9000`
    /// instead). Count MUST equal `--peers`.
    #[arg(long, value_delimiter = ',')]
    validator_ips: Vec<IpAddr>,

    /// How `peers.json` renders each validator's ingress host: `ip` (pinned
    /// docker IP, default) or `dns` (docker service-name hostname
    /// `validator-N:9000`). `dns` requires the validator network's `ALLOW_DNS`
    /// policy to be `true` and all nodes upgraded together.
    #[arg(long, value_enum, env = "PEER_HOST_MODE", default_value_t = PeerHostMode::Ip)]
    peer_host_mode: PeerHostMode,

    /// When set, also emit a CoreDNS `Corefile` + `<zone>.zone` serving the
    /// first `--bootstrappers` validators as `pubkey@validator-N:9000` TXT
    /// records on this zone, for `--dpos.bootstrappers=dns:<zone>`. Unset
    /// (default) writes no DNS files.
    #[arg(long, env = "SEED_DNS_ZONE")]
    seed_dns_zone: Option<String>,

    /// Seat only the first N derived identities (`validator-0..N-1`) as the genesis
    /// committee, and cap `activeValidatorsLength` at N. Unset (default) seats every
    /// identity, which is right where `--peers` IS the committee. The sim/soak derives
    /// an identity POOL larger than the set of nodes it runs, and seating an identity
    /// with no node behind it commits a committee that cannot finalize.
    #[arg(long)]
    committee_size: Option<u32>,

    /// Hand the whole MockBlendToken supply to this validator index's owner key, and
    /// make it the genesis stake sponsor and the stipend reserve. Unset (default)
    /// leaves it on the governance signer, where the token's constructor mints it. The
    /// sim/soak passes `0` because it signs every funding and delegation with
    /// `owner-0.hex`.
    #[arg(long)]
    blend_holder_validator: Option<u32>,

    /// `dposActivationBlock` baked into genesis. Unset (default) schedules
    /// `2 * EPOCH_BLOCK_INTERVAL`, landing the migration anchor in absolute epoch 2.
    /// Pass `0` for the contract's unscheduled sentinel — nothing activates at genesis
    /// and governance sets the real block later, which is what a stand whose bring-up
    /// outlasts block `2 * interval` needs.
    ///
    /// The env var is parsed by hand rather than through clap's `env =` because the
    /// compose files forward it as `${DPOS_ACTIVATION_BLOCK:-}` — deliberately EMPTY when
    /// the host has not tuned it, so the interval-derived default applies. Clap sees an
    /// empty-but-present variable and fails on `cannot parse integer from empty string`,
    /// which is a usage error at genesis-init and a dead stand. Empty means unset here,
    /// as it did when `bootstrap.rs` read the variable itself.
    #[arg(long)]
    dpos_activation_block: Option<u64>,
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

#[derive(clap::Args, Debug)]
struct WriteKeysArgs {
    /// Validator index whose private key files to materialize.
    #[arg(long)]
    idx: u32,

    #[arg(long, default_value = "/runtime")]
    output: PathBuf,

    #[arg(long, default_value_t = 2026)]
    chain_id: u64,

    #[arg(long, env = "FLUENT_DPOS_MNEMONIC", default_value = DEFAULT_MNEMONIC)]
    mnemonic: String,
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
        Cmd::WriteKeys(args) => run_write_keys(args),
    }
}

fn run_write_keys(args: WriteKeysArgs) -> eyre::Result<()> {
    std::fs::create_dir_all(&args.output).wrap_err("create output dir")?;
    // Derive validators[0..=idx] (peers = idx + 1). Derivation is peers-independent
    // (`keys::derive` hashes only seed|role|idx), so 0..POOL-1 come out byte-identical
    // to genesis; write ONLY idx's private files + the grown addresses.json.
    let key_set = keys::derive(&args.mnemonic, args.idx + 1, args.chain_id)?;
    output::write_validator_keys(&args.output, &key_set, args.idx)?;
    tracing::info!(
        idx = args.idx,
        out = %args.output.display(),
        "wrote fresh validator key files (mint-on-demand)"
    );
    Ok(())
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
    eyre::ensure!(
        args.committee_size
            .is_none_or(|n| n >= 1 && n <= args.peers),
        "--committee-size {:?} must be within 1..={} (--peers)",
        args.committee_size,
        args.peers
    );
    eyre::ensure!(
        args.blend_holder_validator.is_none_or(|i| i < args.peers),
        "--blend-holder-validator {:?} out of range for --peers {}",
        args.blend_holder_validator,
        args.peers
    );

    std::fs::create_dir_all(&args.output).wrap_err("create output dir")?;

    let key_set = keys::derive(&args.mnemonic, args.peers, args.chain_id)?;
    tracing::info!(
        peers = args.peers,
        chain_id = args.chain_id,
        bare,
        "keys derived deterministically from mnemonic"
    );

    // The axis is the STAKING MODULE, not predeploys in general: `full` means "staking
    // present and initialized at block 0", `bare` means "no staking", and
    // `artifacts::load` (the only reader of the rWasm blob) is reached from
    // `bootstrap::run` alone. `bare` still installs the Governor, because the module
    // that arrives later by runtime-upgrade compiles `GENESIS_GOVERNANCE` in as the
    // only caller its setters accept and nothing can put a contract at a fixed address
    // after genesis.
    let predeploy_state = if bare {
        let governance_init = artifacts::load_governance(&args.contracts_dir)?;
        tracing::info!(
            contracts_dir = %args.contracts_dir.display(),
            "vendored governance artefact loaded"
        );
        bootstrap::run_governance_only(&key_set, &governance_init, args.chain_id)?
    } else {
        let artefacts = artifacts::load(&args.contracts_dir)?;
        tracing::info!(
            contracts_dir = %args.contracts_dir.display(),
            "vendored forge artefacts loaded"
        );
        let params = bootstrap::BootstrapParams {
            committee_size: args.committee_size.map(|n| n as usize),
            blend_holder: args
                .blend_holder_validator
                .map(|i| key_set.validators[i as usize].l2_signer.address()),
            dpos_activation_block: args.dpos_activation_block.or_else(env_activation_block),
        };
        bootstrap::run(&key_set, &artefacts, args.chain_id, &params)?
    };

    let genesis = genesis::assemble(args.chain_id, &key_set, predeploy_state)?;

    output::write(
        &args.output,
        &genesis,
        &key_set,
        args.bootstrappers as usize,
        &args.validator_ips,
        args.peer_host_mode,
        args.seed_dns_zone.as_deref(),
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
