use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{transaction::eip2718::TypedTransaction, NameOrAddress, TransactionRequest, U64},
};
use flate2::read::GzDecoder;
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, crypto::crypto_keccak256, Address, Bytes, B256,
    PRECOMPILE_BIG_MODEXP, PRECOMPILE_BLAKE2F, PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD, PRECOMPILE_BN256_MUL, PRECOMPILE_BN256_PAIR, PRECOMPILE_EIP2935,
    PRECOMPILE_EIP7951, PRECOMPILE_EVM_RUNTIME, PRECOMPILE_FEE_MANAGER, PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION, PRECOMPILE_NITRO_VERIFIER, PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_RIPEMD160, PRECOMPILE_RUNTIME_UPGRADE, PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256, PRECOMPILE_SVM_RUNTIME, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
    PRECOMPILE_WASM_RUNTIME, PRECOMPILE_WEBAUTHN_VERIFIER, U256, UPDATE_GENESIS_PREFIX,
    WASM_MAX_CODE_SIZE,
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
    io::{Read, Write},
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

    /// Install a locally built .wasm module at one address through upgradeTo(...)
    InstallLocal(InstallLocalArgs),
}

#[derive(Args, Debug)]
struct EndpointArgs {
    /// Use local RPC (http://localhost:8545)
    #[arg(long)]
    local: bool,

    /// Use devnet RPC (https://rpc.devnet.fluent.xyz)
    #[arg(long)]
    dev: bool,

    /// Use testnet RPC (https://rpc.testnet.fluent.xyz)
    #[arg(long)]
    test: bool,

    /// A custom RPC endpoint (overrides --local, --dev, --test)
    #[arg(long)]
    rpc: Option<String>,
}

#[derive(Args, Debug)]
struct CommonArgs {
    /// Genesis release tag, e.g. v0.5.3
    #[arg(long)]
    genesis: String,

    /// Contract key name (e.g. PRECOMPILE_EVM_RUNTIME) from CONTRACTS_TO_UPGRADE.
    /// If omitted, upgrades all known contracts (with a prompt).
    #[arg(long)]
    contract: Option<String>,

    #[command(flatten)]
    endpoint: EndpointArgs,
}

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

#[derive(Args, Debug)]
struct InstallLocalArgs {
    /// Path to the .wasm module to install. The contract compiles it on chain, so this
    /// must be the Wasm source and not an already compiled .rwasm.
    #[arg(long, value_name = "PATH")]
    wasm: PathBuf,

    /// Address to install the module at.
    #[arg(long, value_name = "ADDRESS")]
    target: Address,

    #[command(flatten)]
    endpoint: EndpointArgs,

    #[command(flatten)]
    tx: TxArgs,
}

impl Command {
    /// `InstallLocal` sources its module from disk and never touches a release genesis,
    /// so it carries no `CommonArgs`.
    fn common(&self) -> Option<&CommonArgs> {
        match self {
            Self::DirectUpgrade(args) => Some(&args.common),
            Self::PlanUpgrade(args) => Some(&args.common),
            Self::UpgradePlanned(args) => Some(&args.common),
            Self::InstallLocal(_) => None,
        }
    }
}

/// One module install: the payload that goes on the wire, plus the reference the on-chain
/// module is checked against.
#[derive(Debug)]
struct UpgradeTarget {
    address: Address,
    wasm: Vec<u8>,
    /// The release artefact, when the module came out of a release genesis — matching it
    /// whole also catches a compiler change behind an unchanged Wasm. A locally built
    /// module has no such artefact, so the payload itself is the reference and the check
    /// runs against the hint section, which is where the compiled module keeps it.
    release_module: Option<RwasmModule>,
}

impl UpgradeTarget {
    fn from_release(address: Address, module: RwasmModule) -> Self {
        Self {
            address,
            wasm: module.hint_section.clone(),
            release_module: Some(module),
        }
    }

    fn from_local_wasm(address: Address, wasm: Vec<u8>) -> Self {
        Self {
            address,
            wasm,
            release_module: None,
        }
    }

    fn is_installed(&self, onchain: &RwasmModule) -> bool {
        match &self.release_module {
            Some(module) => onchain == module,
            None => onchain.hint_section == self.wasm,
        }
    }

    fn expected_hash(&self) -> B256 {
        crypto_keccak256(self.wasm.as_slice())
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
struct UpgradeResultManifest {
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
        ("PRECOMPILE_SVM_RUNTIME", PRECOMPILE_SVM_RUNTIME),
        ("PRECOMPILE_WASM_RUNTIME", PRECOMPILE_WASM_RUNTIME),
        ("PRECOMPILE_RUNTIME_UPGRADE", PRECOMPILE_RUNTIME_UPGRADE),
        ("PRECOMPILE_FEE_MANAGER", PRECOMPILE_FEE_MANAGER),
        ("PRECOMPILE_WEBAUTHN_VERIFIER", PRECOMPILE_WEBAUTHN_VERIFIER),
    ])
}

async fn download_genesis_file(genesis_version: &str) -> Result<alloy_genesis::Genesis> {
    let output_file = format!("genesis-{}.json", genesis_version);
    if Path::new(&output_file).exists() {
        let json = fs::read_to_string(&output_file)
            .with_context(|| format!("reading cached {}", output_file))?;
        let result = serde_json::from_str::<alloy_genesis::Genesis>(json.as_str())
            .expect("failed to parse genesis json file");
        return Ok(result);
    }

    let url = format!(
        "https://github.com/fluentlabs-xyz/fluentbase/releases/download/{0}/genesis-{0}.json.gz",
        genesis_version
    );

    print!("Downloading genesis file from {}... ", url);
    std::io::stdout().flush().ok();

    let resp = reqwest::Client::builder()
        .user_agent("fluent-chainspec/1.0")
        .timeout(std::time::Duration::from_secs(60))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    if !resp.status().is_success() {
        bail!("HTTP error! {}", resp.status());
    }
    let bytes = resp.bytes().await?;

    let mut decoder = GzDecoder::new(&bytes[..]);
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .context("gunzip+read_to_string")?;

    fs::write(&output_file, json.as_bytes()).with_context(|| format!("writing {}", output_file))?;
    println!("DONE");

    let result = serde_json::from_str::<alloy_genesis::Genesis>(json.as_str())
        .expect("failed to parse genesis json file");
    Ok(result)
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

fn pick_rpc(args: &EndpointArgs) -> Result<String> {
    if let Some(rpc) = &args.rpc {
        return Ok(rpc.clone());
    }
    let flags = [args.local, args.dev, args.test]
        .into_iter()
        .filter(|x| *x)
        .count();
    if flags != 1 {
        bail!("You must specify exactly one of --local, --dev, or --test");
    }
    Ok(if args.local {
        "http://localhost:8545".to_string()
    } else if args.dev {
        "https://rpc.devnet.fluent.xyz".to_string()
    } else {
        "https://rpc.testnet.fluent.xyz".to_string()
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

fn write_safe_bundle(
    path: &Path,
    genesis_version: &str,
    genesis_hash: B256,
    chain_id: u64,
    updater: Address,
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
        "Fluent runtime upgrade plan bundle\nGenesis version: {}\nGenesis hash: {}\nUpdater: {}\nPlanned upgrades:\n{}",
        genesis_version,
        genesis_hash,
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

fn load_release_targets(
    genesis: &alloy_genesis::Genesis,
    upgrade_list: &[Address],
) -> Result<Vec<UpgradeTarget>> {
    let mut targets = Vec::with_capacity(upgrade_list.len());
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
        targets.push(UpgradeTarget::from_release(*addr, module));
    }
    Ok(targets)
}

/// A compiled `.rwasm` starts with `0xEF52`; the upgrade contract wants the Wasm source and
/// compiles it itself, with the same `compile_rwasm_maybe_system` the genesis path uses, so
/// that both installs land on identical module bytes.
const WASM_MAGIC: [u8; 4] = *b"\0asm";

fn load_local_target(path: &Path, address: Address) -> Result<UpgradeTarget> {
    let wasm = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if !wasm.starts_with(&WASM_MAGIC) {
        bail!(
            "{} is not a Wasm module (no \\0asm magic); the payload must be the .wasm source, \
             not a compiled .rwasm",
            path.display()
        );
    }
    Ok(UpgradeTarget::from_local_wasm(address, wasm))
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

fn preflight_targets(targets: &[UpgradeTarget]) -> Result<()> {
    for target in targets {
        if target.wasm.is_empty() {
            bail!(
                "selected contract {} has an empty Wasm payload",
                target.address
            );
        }
        if target.wasm.len() >= WASM_MAX_CODE_SIZE {
            bail!("selected contract {} exceeds 1MiB", target.address);
        }
    }
    Ok(())
}

/// `None` when the account holds no code — a fresh install target, which the release path
/// never sees but a local one starts from.
async fn onchain_module(
    provider: &Provider<Http>,
    address: Address,
) -> Result<Option<RwasmModule>> {
    let code = provider
        .get_code(NameOrAddress::Address(ethers_address(address)), None)
        .await
        .context("get_code")?;
    if code.is_empty() {
        return Ok(None);
    }
    let (module, _) = RwasmModule::new_checked(code.as_ref())
        .with_context(|| format!("decoding on-chain rwasm for {}", address))?;
    Ok(Some(module))
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

/// `upgradeTo` only re-emits `genesisHash` and `genesisVersion` in `RuntimeUpgraded` and
/// checks neither (contracts/runtime-upgrade/src/lib.rs:115-132). A locally built module
/// belongs to no release genesis, so the hash is zero and the version names the mode.
const LOCAL_GENESIS_HASH: B256 = B256::ZERO;
const LOCAL_GENESIS_VERSION: &str = "local";

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Command::InstallLocal(args) = &cli.command {
        return install_local_module(args).await;
    }
    let common = cli
        .command
        .common()
        .context("release upgrade is missing its genesis arguments")?;

    let genesis = download_genesis_file(&common.genesis).await?;
    let genesis_header = make_genesis_header(&genesis, &FLUENT_HARDFORKS);
    let genesis_hash = genesis_header.hash_slow();

    // Determine which contracts to upgrade.
    let contracts = contracts_to_upgrade();
    let upgrade_list = select_contracts(common, &contracts)?;
    if upgrade_list.is_empty() {
        return Ok(());
    }
    let targets = load_release_targets(&genesis, &upgrade_list)?;
    preflight_targets(&targets)?;

    let rpc = pick_rpc(&common.endpoint)?;
    let provider = Provider::<Http>::try_from(rpc).context("creating provider")?;

    let chain_id = provider
        .get_chainid()
        .await
        .context("get_chainid")?
        .as_u64();

    match &cli.command {
        Command::PlanUpgrade(args) => {
            let mut planned_upgrades = Vec::new();
            for target in targets {
                print!("Planning contract {}... ", target.address);
                std::io::stdout().flush().ok();

                let installed = onchain_module(&provider, target.address).await?;
                if installed.is_some_and(|module| target.is_installed(&module)) {
                    println!("UP-TO-DATE");
                    continue;
                }

                planned_upgrades.push(PlannedUpgrade {
                    contract_key: contract_key_for(&contracts, target.address).to_string(),
                    contract: target.address,
                    wasm_code_hash: target.expected_hash(),
                });
                println!("SAFE_PLAN_QUEUED");
            }

            write_safe_bundle(
                &args.safe_bundle,
                &args.common.genesis,
                genesis_hash,
                chain_id,
                args.updater,
                &planned_upgrades,
            )?;
        }
        Command::DirectUpgrade(args) => {
            run_upgrade_transactions(
                &args.tx,
                &provider,
                chain_id,
                targets,
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
                targets,
                encode_planned_upgrade_call,
            )
            .await?;
        }
        Command::InstallLocal(_) => unreachable!("dispatched before the release path"),
    }

    Ok(())
}

async fn install_local_module(args: &InstallLocalArgs) -> Result<()> {
    let target = load_local_target(&args.wasm, args.target)?;
    let targets = vec![target];
    preflight_targets(&targets)?;

    let rpc = pick_rpc(&args.endpoint)?;
    let provider = Provider::<Http>::try_from(rpc).context("creating provider")?;
    let chain_id = provider
        .get_chainid()
        .await
        .context("get_chainid")?
        .as_u64();

    run_upgrade_transactions(
        &args.tx,
        &provider,
        chain_id,
        targets,
        |contract, wasm_bytecode| {
            encode_direct_upgrade_call(
                contract,
                LOCAL_GENESIS_HASH,
                LOCAL_GENESIS_VERSION,
                wasm_bytecode,
            )
        },
    )
    .await
}

async fn run_upgrade_transactions(
    tx_args: &TxArgs,
    provider: &Provider<Http>,
    chain_id: u64,
    targets: Vec<UpgradeTarget>,
    encode_call: impl Fn(Address, &[u8]) -> Vec<u8>,
) -> Result<()> {
    let wallet = load_wallet(tx_args)?;
    println!("Wallet loaded ({})", wallet.address());
    let wallet = wallet.with_chain_id(chain_id);
    let signer = SignerMiddleware::new(provider.clone(), wallet);
    let mut manifest = UpgradeResultManifest {
        entries: Vec::new(),
    };

    for target in targets {
        print!("Upgrading contract {}... ", target.address);
        std::io::stdout().flush().ok();

        let expected_hash = target.expected_hash();

        if let Some(installed) = onchain_module(provider, target.address).await? {
            if target.is_installed(&installed) {
                manifest.entries.push(UpgradeResultEntry {
                    target: address_hex(target.address),
                    expected_hash: hash_hex(expected_hash),
                    transaction_hash: None,
                    receipt_status: None,
                    verified_onchain_hash: Some(hash_hex(crypto_keccak256(
                        installed.hint_section.as_slice(),
                    ))),
                    result: "up_to_date",
                });
                println!("UP-TO-DATE");
                continue;
            }
        }

        let data = encode_call(target.address, &target.wasm);
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
                    target: address_hex(target.address),
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
                target: address_hex(target.address),
                expected_hash: hash_hex(expected_hash),
                transaction_hash: None,
                receipt_status: None,
                verified_onchain_hash: None,
                result: "raw_tx_printed",
            });
            continue;
        }

        let installed = onchain_module(provider, target.address)
            .await
            .with_context(|| format!("re-reading {} after the upgrade", target.address))?;
        let verified_hash = installed
            .as_ref()
            .map(|module| hash_hex(crypto_keccak256(module.hint_section.as_slice())));
        if !installed.is_some_and(|module| target.is_installed(&module)) {
            manifest.entries.push(UpgradeResultEntry {
                target: address_hex(target.address),
                expected_hash: hash_hex(expected_hash),
                transaction_hash: transaction_hash(outcome),
                receipt_status: receipt_status(outcome),
                verified_onchain_hash: verified_hash.clone(),
                result: "verification_failed",
            });
            print_result_manifest(&manifest)?;
            bail!(
                "post-upgrade bytecode mismatch for {}: verified {}, expected {}",
                target.address,
                verified_hash.as_deref().unwrap_or("no code"),
                hash_hex(expected_hash)
            );
        }
        manifest.entries.push(UpgradeResultEntry {
            target: address_hex(target.address),
            expected_hash: hash_hex(expected_hash),
            transaction_hash: transaction_hash(outcome),
            receipt_status: receipt_status(outcome),
            verified_onchain_hash: verified_hash,
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

fn print_result_manifest(manifest: &UpgradeResultManifest) -> Result<()> {
    let json = serde_json::to_string(manifest).context("serializing result manifest")?;
    println!("RESULT_MANIFEST_JSON={}", json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_fails_when_selected_contract_is_missing() {
        let genesis = alloy_genesis::Genesis::default();
        let err = load_release_targets(&genesis, &[PRECOMPILE_EVM_RUNTIME])
            .expect_err("missing selected contract must fail loading");

        assert!(err
            .to_string()
            .contains("is missing from release artifacts"));
    }

    #[test]
    fn preflight_fails_when_selected_contract_has_empty_payload() {
        let targets = vec![UpgradeTarget::from_local_wasm(
            PRECOMPILE_EVM_RUNTIME,
            Vec::new(),
        )];

        let err = preflight_targets(&targets).expect_err("empty Wasm payload must fail preflight");

        assert!(err.to_string().contains("empty Wasm payload"));
    }

    #[test]
    fn local_load_rejects_a_compiled_rwasm() {
        let path = std::env::temp_dir().join("runtime-upgrade-local-load-rejects.rwasm");
        fs::write(&path, [0xEF, 0x52, 0x00, 0x00]).expect("writing the fixture");

        let err = load_local_target(&path, PRECOMPILE_EVM_RUNTIME)
            .expect_err("a compiled rwasm must be rejected");
        fs::remove_file(&path).ok();

        assert!(err.to_string().contains("is not a Wasm module"));
    }

    #[test]
    fn a_local_target_matches_only_a_module_carrying_the_same_wasm() {
        let wasm = b"\0asm\x01\0\0\0".to_vec();
        let target = UpgradeTarget::from_local_wasm(PRECOMPILE_EVM_RUNTIME, wasm.clone());

        assert!(!target.is_installed(&RwasmModule::default()));
        assert!(target.is_installed(
            &rwasm::RwasmModuleBuilder::default()
                .with_hint_section(&wasm)
                .build()
        ));
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
