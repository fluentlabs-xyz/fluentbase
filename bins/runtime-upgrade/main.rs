use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{transaction::eip2718::TypedTransaction, NameOrAddress, TransactionRequest},
};
use flate2::read::GzDecoder;
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, Address, Bytes, B256, PRECOMPILE_BIG_MODEXP,
    PRECOMPILE_BLAKE2F, PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BLS12_381_G1_MSM,
    PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_MAP_G1,
    PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING, PRECOMPILE_BN256_ADD,
    PRECOMPILE_BN256_MUL, PRECOMPILE_BN256_PAIR, PRECOMPILE_EIP2935, PRECOMPILE_EIP7951,
    PRECOMPILE_EVM_RUNTIME, PRECOMPILE_FEE_MANAGER, PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION, PRECOMPILE_NITRO_VERIFIER, PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_RIPEMD160, PRECOMPILE_RUNTIME_UPGRADE, PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256, PRECOMPILE_SVM_RUNTIME, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
    PRECOMPILE_WASM_RUNTIME, PRECOMPILE_WEBAUTHN_VERIFIER, PRECOMPILE_WRAPPED_ETH, U256,
    UPDATE_GENESIS_PREFIX, WASM_MAX_CODE_SIZE,
};
use reth_chainspec::{
    make_genesis_header, ChainHardforks, EthereumHardfork, ForkCondition, Hardfork,
};
use rpassword::read_password;
use rwasm::RwasmModule;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::Path,
    sync::LazyLock,
};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Genesis release tag, e.g. v0.5.3
    #[arg(long)]
    genesis: String,

    /// Gas limit to use for upgrade transactions
    #[arg(long)]
    gas_limit: Option<u64>,

    /// Contract key name (e.g. EVM_RUNTIME) from CONTRACTS_TO_UPGRADE.
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

    /// A custom RPC endpoint (overrides --local, --dev, --test)
    #[arg(long)]
    rpc: Option<String>,

    /// Private key hex (0x... or raw hex).
    /// If omitted, reads env PRIVATE_KEY. If missing, prompts via hidden input.
    #[arg(long)]
    private_key: Option<String>,

    /// If set: sign tx, print raw tx hex (0x...), and DO NOT broadcast.
    #[arg(long)]
    print_raw_tx: bool,

    /// Use legacy upgrade mode
    #[arg(long, default_value_t = false)]
    force_legacy: bool,

    /// Use legacy upgrade mode
    #[arg(long, default_value_t = false)]
    legacy_prefix: bool,
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
        ("PRECOMPILE_WRAPPED_ETH", PRECOMPILE_WRAPPED_ETH),
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

fn pick_rpc(args: &Args) -> Result<String> {
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

fn load_wallet(args: &Args) -> Result<LocalWallet> {
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
    let args = Args::parse();

    let genesis = download_genesis_file(&args.genesis).await?;
    let genesis_header = make_genesis_header(&genesis, &FLUENT_HARDFORKS);
    let genesis_hash = genesis_header.hash_slow();

    let mut rwasm_module_by_address: HashMap<Address, RwasmModule> = HashMap::new();
    for (addr, entry) in genesis.alloc.iter() {
        let Some(code) = entry.code.as_ref() else {
            continue;
        };
        let Ok((module, _)) = RwasmModule::new_checked(code.as_ref()) else {
            continue;
        };
        if module.hint_section.is_empty() {
            bail!("Failed to extract WASM bytecode from {}", addr);
        }
        rwasm_module_by_address.insert(*addr, module);
    }

    // Determine which contracts to upgrade.
    let contracts = contracts_to_upgrade();
    let upgrade_list: Vec<Address> = match args.contract.as_deref() {
        None => {
            let answer = ask_for("Upgrade ALL known contracts? (Y/n) ")?;
            if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
                return Ok(());
            }
            contracts.values().copied().collect()
        }
        Some(key) => {
            let addr = contracts
                .get(key)
                .ok_or_else(|| anyhow!("Unknown contract: {}", key))?;
            vec![*addr]
        }
    };

    // Wallet from a private key
    let wallet = load_wallet(&args)?;
    println!("Wallet loaded ({})", wallet.address());

    let rpc = pick_rpc(&args)?;
    let provider = Provider::<Http>::try_from(rpc).context("creating provider")?;

    let chain_id = provider
        .get_chainid()
        .await
        .context("get_chainid")?
        .as_u64();

    let wallet = wallet.with_chain_id(chain_id);
    let signer = SignerMiddleware::new(provider.clone(), wallet);
    let signer = std::sync::Arc::new(signer);

    let runtime_upgrade_bytecode = provider
        .get_code(
            NameOrAddress::Address((*PRECOMPILE_RUNTIME_UPGRADE.0).into()),
            None,
        )
        .await?;
    let mut is_legacy_upgrade_scheme = runtime_upgrade_bytecode.is_empty();
    if args.force_legacy {
        is_legacy_upgrade_scheme = true;
    }

    for contract in upgrade_list {
        print!("Upgrading contract {}... ", contract);
        std::io::stdout().flush().ok();

        let new_rwasm: RwasmModule = rwasm_module_by_address
            .get(&contract)
            .cloned()
            .unwrap_or_default();
        if new_rwasm.hint_section.len() >= WASM_MAX_CODE_SIZE {
            println!("FAILED (contract exceeds 1MiB)");
            continue;
        }

        let on_chain_code = provider
            .get_code(NameOrAddress::Address((*contract.0).into()), None)
            .await
            .context("get_code")?;
        let (onchain_rwasm, _) =
            RwasmModule::new_checked(on_chain_code.as_ref()).unwrap_or_default();
        if onchain_rwasm == new_rwasm {
            println!("UP-TO-DATE");
            continue;
        }

        let mut data = vec![];
        if is_legacy_upgrade_scheme {
            data.extend_from_slice(&[0x69, 0xbc, 0x6f, 0x65]);
            data.extend_from_slice(&new_rwasm.hint_section);
        } else {
            data.extend_from_slice(&UPDATE_GENESIS_PREFIX);
            let mut buffer = BytesMut::new();
            if args.legacy_prefix {
                SolidityABI::<(Address, B256, String, Bytes)>::encode(
                    &(
                        contract,
                        genesis_hash,
                        args.genesis.clone(),
                        Bytes::copy_from_slice(&new_rwasm.hint_section),
                    ),
                    &mut buffer,
                    0,
                )
                .unwrap();
            } else {
                SolidityABI::<(Address, B256, String, Bytes)>::encode_function_args(
                    &(
                        contract,
                        genesis_hash,
                        args.genesis.clone(),
                        Bytes::copy_from_slice(&new_rwasm.hint_section),
                    ),
                    &mut buffer,
                )
                .unwrap();
            }
            let buffer = buffer.freeze();
            data.extend_from_slice(buffer.as_ref());
        }

        let send_to = if is_legacy_upgrade_scheme {
            contract
        } else {
            PRECOMPILE_RUNTIME_UPGRADE
        };
        let mut tx = TransactionRequest::new()
            .to(NameOrAddress::Address((*send_to.0).into()))
            .data(data);
        if let Some(gas_limit) = args.gas_limit {
            tx = tx.gas(gas_limit);
        }

        if args.print_raw_tx {
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
            continue;
        }

        // Normal path: broadcast
        match signer.send_transaction(tx, None).await {
            Ok(pending) => {
                let tx_hash = *pending;
                match pending.await {
                    Ok(Some(rcpt)) => {
                        let bn = rcpt.block_number.map(|v| v.as_u64()).unwrap_or_default();
                        println!("DONE (tx_hash={:#x}, block_number={})", tx_hash, bn);
                    }
                    Ok(None) => {
                        println!("DONE (tx_hash={:#x}, block_number=?)", tx_hash);
                    }
                    Err(e) => {
                        println!("FAILED ({})", e);
                        continue;
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("intrinsic gas too low") {
                    println!("FAILED (intrinsic gas too low)");
                } else {
                    println!("FAILED ({})", msg);
                }
                continue;
            }
        }

        let on_chain_code = provider
            .get_code(NameOrAddress::Address((*contract.0).into()), None)
            .await
            .context("get_code")?;
        let (onchain_rwasm, _) =
            RwasmModule::new_checked(on_chain_code.as_ref()).unwrap_or_default();
        if onchain_rwasm != new_rwasm {
            println!(
                " ~ WARNING: upgraded contract bytecode doesn't match: {}, should be {}",
                onchain_rwasm.hint_section.len(),
                new_rwasm.hint_section.len()
            );
        }
    }

    Ok(())
}
