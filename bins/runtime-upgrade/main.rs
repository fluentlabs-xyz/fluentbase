mod provenance;

use crate::provenance::{load_release, ManifestBinding, ReleaseProvenance};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{transaction::eip2718::TypedTransaction, NameOrAddress, TransactionRequest, U64},
};
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, crypto::crypto_keccak256, Address, Bytes, B256,
    PRECOMPILE_BIG_MODEXP, PRECOMPILE_BLAKE2F, PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD, PRECOMPILE_BN256_MUL, PRECOMPILE_BN256_PAIR, PRECOMPILE_EIP2935,
    PRECOMPILE_EIP7951, PRECOMPILE_EVM_RUNTIME, PRECOMPILE_FEE_MANAGER, PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION, PRECOMPILE_NITRO_VERIFIER, PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_RIPEMD160, PRECOMPILE_RUNTIME_UPGRADE, PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, PRECOMPILE_WASM_RUNTIME,
    PRECOMPILE_WEBAUTHN_VERIFIER, U256, UPDATE_GENESIS_PREFIX, WASM_MAX_CODE_SIZE,
};
use reth_chainspec::{
    make_genesis_header, ChainHardforks, EthereumHardfork, ForkCondition, Hardfork,
};
use rpassword::read_password;
use rwasm::RwasmModule;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Immediately upgrade contracts through upgradeTo(...)
    DirectUpgrade(DirectUpgradeArgs),

    /// Build a Safe bundle that plans approved target/hash pairs through planUpgrade(...)
    PlanUpgrade(PlanUpgradeArgs),

    /// Execute previously planned contracts through upgradeToPlanned(...)
    UpgradePlanned(UpgradePlannedArgs),
}

#[derive(Args, Debug)]
struct CommonArgs {
    /// Genesis release tag, e.g. v0.5.3
    #[arg(long)]
    genesis: String,

    /// Release channel of the genesis asset (e.g. `mainnet`). Omit for the default asset.
    /// This selects which published artifact is authenticated, so it must match the network the
    /// upgrade targets. Implied by --mainnet, which is why the two cannot be combined.
    #[arg(long, conflicts_with = "mainnet")]
    genesis_channel: Option<String>,

    /// Contract key name (e.g. PRECOMPILE_EVM_RUNTIME) from CONTRACTS_TO_UPGRADE.
    /// If omitted, upgrades all known contracts (with a prompt).
    #[arg(long)]
    contract: Option<String>,

    /// Use local RPC (http://localhost:8545)
    #[arg(long)]
    local: bool,

    /// Use devnet RPC (https://rpc.devnet.fluent.xyz)
    #[arg(long)]
    dev: bool,

    /// Use testnet RPC (https://rpc.testnet.fluent.xyz)
    #[arg(long)]
    test: bool,

    /// Use mainnet RPC (https://rpc.fluent.xyz) and the `mainnet` genesis asset
    #[arg(long)]
    mainnet: bool,

    /// A custom RPC endpoint. Also select its network with --local/--dev/--test/--mainnet, or pass
    /// --genesis-channel explicitly, so the authenticated genesis choice is never implicit.
    #[arg(long)]
    rpc: Option<String>,
}

impl CommonArgs {
    /// Release channel of the genesis asset to authenticate.
    ///
    /// `--mainnet` implies the `mainnet` channel so the artifact cannot silently disagree with the
    /// network being upgraded; clap rejects passing both.
    fn genesis_channel(&self) -> Option<&str> {
        match self.genesis_channel.as_deref() {
            Some(channel) => Some(channel),
            None if self.mainnet => Some(MAINNET_GENESIS_CHANNEL),
            None => None,
        }
    }
}

/// Genesis release channel for Fluent Mainnet artifacts.
const MAINNET_GENESIS_CHANNEL: &str = "mainnet";

#[derive(Args, Debug)]
struct TxArgs {
    /// Gas limit to use for upgrade transactions
    #[arg(long)]
    gas_limit: Option<u64>,

    /// Private key hex (0x... or raw hex).
    /// If omitted, reads env PRIVATE_KEY. If missing, prompts via hidden input.
    #[arg(long)]
    private_key: Option<String>,

    /// If set: sign tx, print raw tx hex (0x...), and DO NOT broadcast.
    #[arg(long)]
    print_raw_tx: bool,
}

#[derive(Args, Debug)]
struct DirectUpgradeArgs {
    #[command(flatten)]
    common: CommonArgs,

    #[command(flatten)]
    tx: TxArgs,
}

#[derive(Args, Debug)]
struct PlanUpgradeArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Authorized updater address for a planned runtime upgrade.
    #[arg(long, value_name = "ADDRESS")]
    updater: Address,

    /// Write Safe Transaction Builder JSON and DO NOT sign or broadcast.
    #[arg(long, value_name = "PATH")]
    safe_bundle: PathBuf,
}

#[derive(Args, Debug)]
struct UpgradePlannedArgs {
    #[command(flatten)]
    common: CommonArgs,

    #[command(flatten)]
    tx: TxArgs,
}

impl Command {
    fn common(&self) -> &CommonArgs {
        match self {
            Self::DirectUpgrade(args) => &args.common,
            Self::PlanUpgrade(args) => &args.common,
            Self::UpgradePlanned(args) => &args.common,
        }
    }
}

struct PlannedUpgrade {
    contract_key: String,
    contract: Address,
    wasm_code_hash: B256,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransactionOutcome {
    Printed,
    Mined {
        tx_hash: ethers::types::H256,
        receipt_status: u64,
    },
}

#[derive(Serialize)]
struct UpgradeResultManifest<'a> {
    /// What the release artifact these payloads came from was proven to be.
    provenance: &'a ReleaseProvenance,
    entries: Vec<UpgradeResultEntry>,
}

#[derive(Serialize)]
struct UpgradeResultEntry {
    target: String,
    expected_hash: String,
    transaction_hash: Option<String>,
    receipt_status: Option<u64>,
    verified_onchain_hash: Option<String>,
    result: &'static str,
}

#[derive(Serialize)]
struct SafeBundle {
    version: &'static str,
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "createdAt")]
    created_at: u128,
    meta: SafeBundleMeta,
    transactions: Vec<SafeBundleTransaction>,
}

#[derive(Serialize)]
struct SafeBundleMeta {
    name: String,
    description: String,
    #[serde(rename = "txBuilderVersion")]
    tx_builder_version: String,
    #[serde(rename = "createdFromSafeAddress")]
    created_from_safe_address: String,
    #[serde(rename = "createdFromOwnerAddress")]
    created_from_owner_address: String,
    checksum: String,
}

#[derive(Serialize)]
struct SafeBundleTransaction {
    to: String,
    value: &'static str,
    data: String,
    #[serde(rename = "contractMethod")]
    contract_method: Option<serde_json::Value>,
    #[serde(rename = "contractInputsValues")]
    contract_inputs_values: Option<serde_json::Value>,
}

fn contracts_to_upgrade() -> HashMap<&'static str, Address> {
    HashMap::from([
        ("PRECOMPILE_BIG_MODEXP", PRECOMPILE_BIG_MODEXP),
        ("PRECOMPILE_BLAKE2F", PRECOMPILE_BLAKE2F),
        ("PRECOMPILE_BLS12_381_G1_ADD", PRECOMPILE_BLS12_381_G1_ADD),
        ("PRECOMPILE_BLS12_381_G1_MSM", PRECOMPILE_BLS12_381_G1_MSM),
        ("PRECOMPILE_BLS12_381_G2_ADD", PRECOMPILE_BLS12_381_G2_ADD),
        ("PRECOMPILE_BLS12_381_G2_MSM", PRECOMPILE_BLS12_381_G2_MSM),
        ("PRECOMPILE_BLS12_381_MAP_G1", PRECOMPILE_BLS12_381_MAP_G1),
        ("PRECOMPILE_BLS12_381_MAP_G2", PRECOMPILE_BLS12_381_MAP_G2),
        ("PRECOMPILE_BLS12_381_PAIRING", PRECOMPILE_BLS12_381_PAIRING),
        ("PRECOMPILE_BN256_ADD", PRECOMPILE_BN256_ADD),
        ("PRECOMPILE_BN256_MUL", PRECOMPILE_BN256_MUL),
        ("PRECOMPILE_BN256_PAIR", PRECOMPILE_BN256_PAIR),
        ("PRECOMPILE_EIP2935", PRECOMPILE_EIP2935),
        ("PRECOMPILE_EIP7951", PRECOMPILE_EIP7951),
        (
            "PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME",
            PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ),
        ("PRECOMPILE_EVM_RUNTIME", PRECOMPILE_EVM_RUNTIME),
        ("PRECOMPILE_IDENTITY", PRECOMPILE_IDENTITY),
        (
            "PRECOMPILE_KZG_POINT_EVALUATION",
            PRECOMPILE_KZG_POINT_EVALUATION,
        ),
        ("PRECOMPILE_NITRO_VERIFIER", PRECOMPILE_NITRO_VERIFIER),
        ("PRECOMPILE_OAUTH2_VERIFIER", PRECOMPILE_OAUTH2_VERIFIER),
        ("PRECOMPILE_RIPEMD160", PRECOMPILE_RIPEMD160),
        ("PRECOMPILE_SECP256K1_RECOVER", PRECOMPILE_SECP256K1_RECOVER),
        ("PRECOMPILE_SHA256", PRECOMPILE_SHA256),
        ("PRECOMPILE_WASM_RUNTIME", PRECOMPILE_WASM_RUNTIME),
        ("PRECOMPILE_RUNTIME_UPGRADE", PRECOMPILE_RUNTIME_UPGRADE),
        ("PRECOMPILE_FEE_MANAGER", PRECOMPILE_FEE_MANAGER),
        ("PRECOMPILE_WEBAUTHN_VERIFIER", PRECOMPILE_WEBAUTHN_VERIFIER),
    ])
}

/// Prints what the artifact's provenance was proven to be, before anything privileged happens.
fn report_provenance(provenance: &ReleaseProvenance) {
    println!(
        "Using {} from release {} (sha256 {})",
        provenance.asset, provenance.tag, provenance.sha256
    );
    match &provenance.manifest {
        ManifestBinding::Verified => {
            let commit = provenance.commit.as_deref().unwrap_or("unknown");
            println!("  provenance: signed release manifest verified (commit {commit})");
        }
        ManifestBinding::Unavailable { reason } => {
            println!("  provenance: detached signature verified");
            eprintln!(
                "  WARNING: release {} publishes no signed digest manifest ({reason}).\n\
                 \x20          The artifact is bound by its detached signature only — there is no\n\
                 \x20          independent binding of asset name and digest to this release.",
                provenance.tag
            );
        }
    }
}

fn ask_for(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn ask_for_secret(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    std::io::stdout().flush().ok();
    let s = read_password().expect("Failed to read secret");
    Ok(s)
}

fn pick_rpc(args: &CommonArgs) -> Result<String> {
    let flags = [args.local, args.dev, args.test, args.mainnet]
        .into_iter()
        .filter(|x| *x)
        .count();
    if let Some(rpc) = &args.rpc {
        if flags > 1 {
            bail!("You may select at most one of --local, --dev, --test, or --mainnet with --rpc");
        }
        if flags == 0 && args.genesis_channel.is_none() {
            bail!(
                "--rpc requires an explicit network flag or --genesis-channel so the genesis \
                 asset is not selected implicitly"
            );
        }
        return Ok(rpc.clone());
    }
    if flags != 1 {
        bail!("You must specify exactly one of --local, --dev, --test, or --mainnet");
    }
    Ok(if args.local {
        "http://localhost:8545".to_string()
    } else if args.dev {
        "https://rpc.devnet.fluent.xyz".to_string()
    } else if args.test {
        "https://rpc.testnet.fluent.xyz".to_string()
    } else {
        "https://rpc.fluent.xyz".to_string()
    })
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

fn ethers_address(address: Address) -> ethers::types::Address {
    (*address.0).into()
}

fn address_hex(address: Address) -> String {
    format!("{:#x}", ethers_address(address))
}

fn hash_hex(hash: B256) -> String {
    format!("0x{}", hex::encode(hash))
}

fn contract_key_for(contracts: &HashMap<&'static str, Address>, contract: Address) -> &'static str {
    contracts
        .iter()
        .find_map(|(key, address)| (*address == contract).then_some(*key))
        .unwrap_or("UNKNOWN")
}

const PLAN_UPGRADE_PREFIX: [u8; 4] = [0x50, 0xc9, 0xc6, 0x68];
const UPGRADE_TO_PLANNED_SIGNATURE: &[u8] = b"upgradeToPlanned(address,bytes)";

#[allow(clippy::too_many_arguments)]
fn write_safe_bundle(
    path: &Path,
    genesis_version: &str,
    genesis_hash: B256,
    chain_id: u64,
    updater: Address,
    provenance: &ReleaseProvenance,
    planned_upgrades: &[PlannedUpgrade],
) -> Result<()> {
    if planned_upgrades.is_empty() {
        bail!("no runtime upgrades need planning");
    }

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before UNIX_EPOCH")?
        .as_millis();
    let metadata = planned_upgrades
        .iter()
        .map(|upgrade| {
            format!(
                "{}: contract={}, wasm_hash={}",
                upgrade.contract_key,
                address_hex(upgrade.contract),
                hash_hex(upgrade.wasm_code_hash),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let description = format!(
        "Fluent runtime upgrade plan bundle\nGenesis version: {}\nGenesis hash: {}\nGenesis artifact: {} (sha256 {})\nArtifact provenance: {}\nUpdater: {}\nPlanned upgrades:\n{}",
        genesis_version,
        genesis_hash,
        provenance.asset,
        provenance.sha256,
        match &provenance.manifest {
            ManifestBinding::Verified => "signed release manifest verified".to_string(),
            ManifestBinding::Unavailable { .. } =>
                "detached signature only (release publishes no manifest)".to_string(),
        },
        address_hex(updater),
        metadata
    );

    let target_addresses = planned_upgrades
        .iter()
        .map(|upgrade| upgrade.contract)
        .collect::<Vec<_>>();
    let wasm_code_hashes = planned_upgrades
        .iter()
        .map(|upgrade| upgrade.wasm_code_hash)
        .collect::<Vec<_>>();

    let mut data = Vec::from(PLAN_UPGRADE_PREFIX);
    let mut buffer = BytesMut::new();
    SolidityABI::<(B256, String, Vec<Address>, Vec<B256>, Address)>::encode_function_args(
        &(
            genesis_hash,
            genesis_version.to_string(),
            target_addresses,
            wasm_code_hashes,
            updater,
        ),
        &mut buffer,
    )
    .unwrap();
    let buffer = buffer.freeze();
    data.extend_from_slice(buffer.as_ref());

    let transactions = vec![SafeBundleTransaction {
        to: address_hex(PRECOMPILE_RUNTIME_UPGRADE),
        value: "0",
        data: format!("0x{}", hex::encode(&data)),
        contract_method: None,
        contract_inputs_values: None,
    }];
    let bundle = SafeBundle {
        version: "1.0",
        chain_id: chain_id.to_string(),
        created_at,
        meta: SafeBundleMeta {
            name: format!("Fluent runtime upgrade plan {}", genesis_version),
            description,
            tx_builder_version: "1.18.0".to_string(),
            created_from_safe_address: String::new(),
            created_from_owner_address: String::new(),
            checksum: String::new(),
        },
        transactions,
    };
    let json = serde_json::to_string_pretty(&bundle).context("serializing Safe bundle")?;
    if path == Path::new("-") {
        println!("{}", json);
    } else {
        fs::write(path, format!("{}\n", json))
            .with_context(|| format!("writing Safe bundle {}", path.display()))?;
        println!("SAFE_BUNDLE={}", path.display());
    }
    Ok(())
}

fn load_wallet(args: &TxArgs) -> Result<LocalWallet> {
    // Priority: CLI flag -> env -> prompt (hidden)
    let pk = if let Some(pk) = args.private_key.as_deref() {
        pk.to_string()
    } else if let Ok(pk) = std::env::var("PRIVATE_KEY") {
        pk
    } else {
        ask_for_secret("Enter private key (hex, hidden input): ")?
    };
    let pk = strip_0x(&pk);
    let bytes = hex::decode(pk).context("private key hex decode")?;
    if bytes.len() != 32 {
        bail!("private key must be 32 bytes (got {})", bytes.len());
    }
    LocalWallet::from_bytes(&bytes).context("creating wallet")
}

fn function_selector(signature: &[u8]) -> [u8; 4] {
    let hash = crypto_keccak256(signature);
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&hash.as_slice()[..4]);
    selector
}

fn load_release_modules(
    genesis: &alloy_genesis::Genesis,
    upgrade_list: &[Address],
) -> Result<HashMap<Address, RwasmModule>> {
    let mut rwasm_module_by_address: HashMap<Address, RwasmModule> = HashMap::new();
    for addr in upgrade_list {
        let entry = genesis.alloc.get(addr).ok_or_else(|| {
            anyhow!(
                "selected contract {} is missing from release artifacts",
                addr
            )
        })?;
        let code = entry
            .code
            .as_ref()
            .ok_or_else(|| anyhow!("selected contract {} has no release bytecode", addr))?;
        let (module, _) = RwasmModule::new_checked(code.as_ref())
            .with_context(|| format!("malformed rwasm artifact in genesis allocation {}", addr))?;
        if module.hint_section.is_empty() {
            bail!("Failed to extract WASM bytecode from {}", addr);
        }
        rwasm_module_by_address.insert(*addr, module);
    }
    Ok(rwasm_module_by_address)
}

fn select_contracts(
    args: &CommonArgs,
    contracts: &HashMap<&'static str, Address>,
) -> Result<Vec<Address>> {
    match args.contract.as_deref() {
        None => {
            let answer = ask_for("Upgrade ALL known contracts? (Y/n) ")?;
            if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
                return Ok(Vec::new());
            }
            Ok(contracts.values().copied().collect())
        }
        Some(key) => {
            let addr = contracts
                .get(key)
                .ok_or_else(|| anyhow!("Unknown contract: {}", key))?;
            Ok(vec![*addr])
        }
    }
}

fn preflight_selected_modules(
    rwasm_module_by_address: &HashMap<Address, RwasmModule>,
    upgrade_list: &[Address],
) -> Result<()> {
    for contract in upgrade_list {
        let module = rwasm_module_by_address.get(contract).ok_or_else(|| {
            anyhow!(
                "selected contract {} is missing from release artifacts",
                contract
            )
        })?;
        if module.hint_section.is_empty() {
            bail!(
                "selected contract {} has an empty Wasm hint section",
                contract
            );
        }
        if module.hint_section.len() >= WASM_MAX_CODE_SIZE {
            bail!("selected contract {} exceeds 1MiB", contract);
        }
    }
    Ok(())
}

fn encode_direct_upgrade_call(
    contract: Address,
    genesis_hash: B256,
    genesis_version: &str,
    wasm_bytecode: &[u8],
) -> Vec<u8> {
    let mut data = Vec::from(UPDATE_GENESIS_PREFIX);
    let mut buffer = BytesMut::new();
    SolidityABI::<(Address, B256, String, Bytes)>::encode_function_args(
        &(
            contract,
            genesis_hash,
            genesis_version.to_string(),
            Bytes::copy_from_slice(wasm_bytecode),
        ),
        &mut buffer,
    )
    .unwrap();
    let buffer = buffer.freeze();
    data.extend_from_slice(buffer.as_ref());
    data
}

fn encode_planned_upgrade_call(contract: Address, wasm_bytecode: &[u8]) -> Vec<u8> {
    let mut data = Vec::from(function_selector(UPGRADE_TO_PLANNED_SIGNATURE));
    let mut buffer = BytesMut::new();
    SolidityABI::<(Address, Bytes)>::encode_function_args(
        &(contract, Bytes::copy_from_slice(wasm_bytecode)),
        &mut buffer,
    )
    .unwrap();
    let buffer = buffer.freeze();
    data.extend_from_slice(buffer.as_ref());
    data
}

async fn send_runtime_upgrade_tx(
    signer: &SignerMiddleware<Provider<Http>, LocalWallet>,
    tx: TransactionRequest,
    print_raw_tx: bool,
) -> Result<TransactionOutcome> {
    if print_raw_tx {
        let mut typed: TypedTransaction = tx.into();
        signer
            .fill_transaction(&mut typed, None)
            .await
            .context("fill_transaction")?;
        let sig = signer
            .signer()
            .sign_transaction(&typed)
            .await
            .context("sign_transaction")?;
        let raw = typed.rlp_signed(&sig);
        println!("RAW_TX=0x{}", hex::encode(raw));
        return Ok(TransactionOutcome::Printed);
    }

    match signer.send_transaction(tx, None).await {
        Ok(pending) => {
            let tx_hash = *pending;
            let rcpt = pending
                .await
                .with_context(|| format!("waiting for receipt for tx {:#x}", tx_hash))?
                .ok_or_else(|| anyhow!("missing receipt for tx {:#x}", tx_hash))?;
            let status = receipt_success_status(rcpt.status)
                .with_context(|| format!("tx {:#x} did not succeed", tx_hash))?;
            let bn = rcpt.block_number.map(|v| v.as_u64()).unwrap_or_default();
            println!(
                "DONE (tx_hash={:#x}, block_number={}, status={})",
                tx_hash, bn, status
            );
            Ok(TransactionOutcome::Mined {
                tx_hash,
                receipt_status: status,
            })
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("intrinsic gas too low") {
                bail!("send_transaction failed: intrinsic gas too low");
            } else {
                bail!("send_transaction failed: {}", msg);
            }
        }
    }
}

fn receipt_success_status(status: Option<U64>) -> Result<u64> {
    match status.map(|v| v.as_u64()) {
        Some(1) => Ok(1),
        Some(value) => bail!("receipt status is {}", value),
        None => bail!("receipt status is missing"),
    }
}

pub static FLUENT_HARDFORKS: LazyLock<ChainHardforks> = LazyLock::new(|| {
    ChainHardforks::new(vec![
        (EthereumHardfork::Frontier.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Homestead.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Dao.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Tangerine.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::SpuriousDragon.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Byzantium.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::Constantinople.boxed(),
            ForkCondition::Block(0),
        ),
        (
            EthereumHardfork::Petersburg.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Istanbul.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Berlin.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::London.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::Paris.boxed(),
            ForkCondition::TTD {
                activation_block_number: 0,
                fork_block: None,
                total_difficulty: U256::ZERO,
            },
        ),
        (
            EthereumHardfork::Shanghai.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (
            EthereumHardfork::Cancun.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (
            EthereumHardfork::Prague.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (EthereumHardfork::Osaka.boxed(), ForkCondition::Timestamp(0)),
    ])
});

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let common = cli.command.common();

    // Provenance first: nothing below this point may run against an unauthenticated artifact, and
    // the operator wallet is not touched until it has passed.
    let release = load_release(
        &common.genesis,
        common.genesis_channel(),
        &provenance::cache_dir(),
    )
    .await?;
    report_provenance(&release.provenance);
    let provenance = release.provenance;
    let genesis = release.genesis;
    let genesis_header = make_genesis_header(&genesis, &FLUENT_HARDFORKS);
    let genesis_hash = genesis_header.hash_slow();

    // Determine which contracts to upgrade.
    let contracts = contracts_to_upgrade();
    let mut upgrade_list = select_contracts(common, &contracts)?;
    if upgrade_list.is_empty() {
        return Ok(());
    }
    upgrade_list.sort();
    let rwasm_module_by_address = load_release_modules(&genesis, &upgrade_list)?;
    preflight_selected_modules(&rwasm_module_by_address, &upgrade_list)?;

    let rpc = pick_rpc(common)?;
    let provider = Provider::<Http>::try_from(rpc).context("creating provider")?;

    let chain_id = provider
        .get_chainid()
        .await
        .context("get_chainid")?
        .as_u64();

    match &cli.command {
        Command::PlanUpgrade(args) => {
            let mut planned_upgrades = Vec::new();
            for contract in upgrade_list {
                print!("Planning contract {}... ", contract);
                std::io::stdout().flush().ok();

                let new_rwasm = rwasm_module_by_address
                    .get(&contract)
                    .expect("selected modules were preflighted");

                let on_chain_code = provider
                    .get_code(NameOrAddress::Address((*contract.0).into()), None)
                    .await
                    .context("get_code")?;
                let (onchain_rwasm, _) = RwasmModule::new_checked(on_chain_code.as_ref())
                    .with_context(|| format!("decoding on-chain rwasm for {}", contract))?;
                if &onchain_rwasm == new_rwasm {
                    println!("UP-TO-DATE");
                    continue;
                }

                planned_upgrades.push(PlannedUpgrade {
                    contract_key: contract_key_for(&contracts, contract).to_string(),
                    contract,
                    wasm_code_hash: crypto_keccak256(new_rwasm.hint_section.as_slice()),
                });
                println!("SAFE_PLAN_QUEUED");
            }

            write_safe_bundle(
                &args.safe_bundle,
                &args.common.genesis,
                genesis_hash,
                chain_id,
                args.updater,
                &provenance,
                &planned_upgrades,
            )?;
        }
        Command::DirectUpgrade(args) => {
            run_upgrade_transactions(
                &args.tx,
                &provider,
                chain_id,
                &provenance,
                &rwasm_module_by_address,
                upgrade_list,
                |contract, wasm_bytecode| {
                    encode_direct_upgrade_call(
                        contract,
                        genesis_hash,
                        &args.common.genesis,
                        wasm_bytecode,
                    )
                },
            )
            .await?;
        }
        Command::UpgradePlanned(args) => {
            run_upgrade_transactions(
                &args.tx,
                &provider,
                chain_id,
                &provenance,
                &rwasm_module_by_address,
                upgrade_list,
                encode_planned_upgrade_call,
            )
            .await?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_upgrade_transactions(
    tx_args: &TxArgs,
    provider: &Provider<Http>,
    chain_id: u64,
    provenance: &ReleaseProvenance,
    rwasm_module_by_address: &HashMap<Address, RwasmModule>,
    upgrade_list: Vec<Address>,
    encode_call: impl Fn(Address, &[u8]) -> Vec<u8>,
) -> Result<()> {
    let wallet = load_wallet(tx_args)?;
    println!("Wallet loaded ({})", wallet.address());
    let wallet = wallet.with_chain_id(chain_id);
    let signer = SignerMiddleware::new(provider.clone(), wallet);
    let mut manifest = UpgradeResultManifest {
        provenance,
        entries: Vec::new(),
    };

    for contract in upgrade_list {
        print!("Upgrading contract {}... ", contract);
        std::io::stdout().flush().ok();

        let new_rwasm = rwasm_module_by_address
            .get(&contract)
            .expect("selected modules were preflighted");
        let expected_hash = crypto_keccak256(new_rwasm.hint_section.as_slice());

        let on_chain_code = provider
            .get_code(NameOrAddress::Address((*contract.0).into()), None)
            .await
            .context("get_code")?;
        let (onchain_rwasm, _) = RwasmModule::new_checked(on_chain_code.as_ref())
            .with_context(|| format!("decoding on-chain rwasm for {}", contract))?;
        if &onchain_rwasm == new_rwasm {
            manifest.entries.push(UpgradeResultEntry {
                target: address_hex(contract),
                expected_hash: hash_hex(expected_hash),
                transaction_hash: None,
                receipt_status: None,
                verified_onchain_hash: Some(hash_hex(crypto_keccak256(
                    onchain_rwasm.hint_section.as_slice(),
                ))),
                result: "up_to_date",
            });
            println!("UP-TO-DATE");
            continue;
        }

        let data = encode_call(contract, &new_rwasm.hint_section);
        let mut tx = TransactionRequest::new()
            .to(NameOrAddress::Address(
                (*PRECOMPILE_RUNTIME_UPGRADE.0).into(),
            ))
            .data(data);
        if let Some(gas_limit) = tx_args.gas_limit {
            tx = tx.gas(gas_limit);
        }

        let outcome = match send_runtime_upgrade_tx(&signer, tx, tx_args.print_raw_tx).await {
            Ok(outcome) => outcome,
            Err(error) => {
                manifest.entries.push(UpgradeResultEntry {
                    target: address_hex(contract),
                    expected_hash: hash_hex(expected_hash),
                    transaction_hash: None,
                    receipt_status: None,
                    verified_onchain_hash: None,
                    result: "failed",
                });
                print_result_manifest(&manifest)?;
                return Err(error);
            }
        };
        if outcome == TransactionOutcome::Printed {
            manifest.entries.push(UpgradeResultEntry {
                target: address_hex(contract),
                expected_hash: hash_hex(expected_hash),
                transaction_hash: None,
                receipt_status: None,
                verified_onchain_hash: None,
                result: "raw_tx_printed",
            });
            continue;
        }

        let on_chain_code = provider
            .get_code(NameOrAddress::Address((*contract.0).into()), None)
            .await
            .context("get_code")?;
        let (onchain_rwasm, _) = RwasmModule::new_checked(on_chain_code.as_ref())
            .with_context(|| format!("decoding post-upgrade on-chain rwasm for {}", contract))?;
        let verified_hash = crypto_keccak256(onchain_rwasm.hint_section.as_slice());
        if &onchain_rwasm != new_rwasm {
            manifest.entries.push(UpgradeResultEntry {
                target: address_hex(contract),
                expected_hash: hash_hex(expected_hash),
                transaction_hash: transaction_hash(outcome),
                receipt_status: receipt_status(outcome),
                verified_onchain_hash: Some(hash_hex(verified_hash)),
                result: "verification_failed",
            });
            print_result_manifest(&manifest)?;
            bail!(
                "post-upgrade bytecode mismatch for {}: verified {}, expected {}",
                contract,
                hash_hex(verified_hash),
                hash_hex(expected_hash)
            );
        }
        manifest.entries.push(UpgradeResultEntry {
            target: address_hex(contract),
            expected_hash: hash_hex(expected_hash),
            transaction_hash: transaction_hash(outcome),
            receipt_status: receipt_status(outcome),
            verified_onchain_hash: Some(hash_hex(verified_hash)),
            result: "upgraded",
        });
    }

    print_result_manifest(&manifest)?;
    Ok(())
}

fn transaction_hash(outcome: TransactionOutcome) -> Option<String> {
    match outcome {
        TransactionOutcome::Printed => None,
        TransactionOutcome::Mined { tx_hash, .. } => Some(format!("{:#x}", tx_hash)),
    }
}

fn receipt_status(outcome: TransactionOutcome) -> Option<u64> {
    match outcome {
        TransactionOutcome::Printed => None,
        TransactionOutcome::Mined { receipt_status, .. } => Some(receipt_status),
    }
}

fn print_result_manifest(manifest: &UpgradeResultManifest<'_>) -> Result<()> {
    let json = serde_json::to_string(manifest).context("serializing result manifest")?;
    println!("RESULT_MANIFEST_JSON={}", json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common_args(argv: &[&str]) -> CommonArgs {
        #[derive(Parser, Debug)]
        struct Harness {
            #[command(flatten)]
            common: CommonArgs,
        }
        let mut full = vec!["runtime-upgrade", "--genesis", "v1.3.2"];
        full.extend_from_slice(argv);
        Harness::try_parse_from(full)
            .unwrap_or_else(|err| panic!("parsing {argv:?}: {err}"))
            .common
    }

    #[test]
    fn mainnet_flag_selects_the_mainnet_rpc_and_genesis_channel() {
        let args = common_args(&["--mainnet"]);
        assert_eq!(pick_rpc(&args).unwrap(), "https://rpc.fluent.xyz");
        assert_eq!(args.genesis_channel(), Some("mainnet"));
    }

    #[test]
    fn other_networks_keep_the_default_genesis_channel() {
        for (flag, rpc) in [
            ("--local", "http://localhost:8545"),
            ("--dev", "https://rpc.devnet.fluent.xyz"),
            ("--test", "https://rpc.testnet.fluent.xyz"),
        ] {
            let args = common_args(&[flag]);
            assert_eq!(pick_rpc(&args).unwrap(), rpc, "{flag}");
            assert_eq!(args.genesis_channel(), None, "{flag}");
        }
    }

    #[test]
    fn genesis_channel_can_be_set_explicitly_without_mainnet() {
        // A custom mainnet RPC still needs the mainnet artifact, so the two stay separable.
        let args = common_args(&[
            "--rpc",
            "https://internal.example",
            "--genesis-channel",
            "mainnet",
        ]);
        assert_eq!(pick_rpc(&args).unwrap(), "https://internal.example");
        assert_eq!(args.genesis_channel(), Some("mainnet"));
    }

    #[test]
    fn custom_rpc_requires_an_explicit_genesis_selection() {
        let err = pick_rpc(&common_args(&["--rpc", "https://internal.example"]))
            .expect_err("a custom RPC must not silently select the default genesis asset");
        assert!(err.to_string().contains("genesis"), "{err:#}");

        let args = common_args(&["--rpc", "https://internal.example", "--dev"]);
        assert_eq!(pick_rpc(&args).unwrap(), "https://internal.example");
        assert_eq!(args.genesis_channel(), None);
    }

    #[test]
    fn mainnet_cannot_be_combined_with_an_explicit_genesis_channel() {
        // Letting these disagree would authenticate one network's artifact for another's upgrade.
        #[derive(Parser, Debug)]
        struct Harness {
            #[command(flatten)]
            common: CommonArgs,
        }
        Harness::try_parse_from([
            "runtime-upgrade",
            "--genesis",
            "v1.3.2",
            "--mainnet",
            "--genesis-channel",
            "devnet",
        ])
        .expect_err("conflicting channel selection must be rejected");
    }

    #[test]
    fn exactly_one_network_flag_is_required() {
        pick_rpc(&common_args(&[])).expect_err("no network flag must be rejected");
        pick_rpc(&common_args(&["--mainnet", "--dev"]))
            .expect_err("two network flags must be rejected");
    }

    #[test]
    fn preflight_fails_when_selected_contract_is_missing() {
        let modules = HashMap::new();
        let err = preflight_selected_modules(&modules, &[PRECOMPILE_EVM_RUNTIME])
            .expect_err("missing selected contract must fail preflight");

        assert!(err
            .to_string()
            .contains("is missing from release artifacts"));
    }

    #[test]
    fn preflight_fails_when_selected_contract_has_empty_hint_section() {
        let mut modules = HashMap::new();
        modules.insert(PRECOMPILE_EVM_RUNTIME, RwasmModule::default());

        let err = preflight_selected_modules(&modules, &[PRECOMPILE_EVM_RUNTIME])
            .expect_err("empty hint section must fail preflight");

        assert!(err.to_string().contains("empty Wasm hint section"));
    }

    #[test]
    fn reverted_receipt_fails() {
        let err =
            receipt_success_status(Some(U64::zero())).expect_err("reverted receipt must fail");

        assert!(err.to_string().contains("receipt status is 0"));
    }

    #[test]
    fn missing_receipt_status_fails() {
        let err = receipt_success_status(None).expect_err("missing receipt status must fail");

        assert!(err.to_string().contains("receipt status is missing"));
    }

    #[test]
    fn successful_receipt_status_is_returned() {
        assert_eq!(receipt_success_status(Some(U64::one())).unwrap(), 1);
    }
}
