use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fluentbase_build::{execute_build, Artifact, BuildArgs};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

/// Fluentbase CLI for building and verifying smart contracts
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a Fluentbase smart contract
    Build(BuildCommand),
    /// Verify a deployed smart contract
    Verify(VerifyCommand),
}

/// Build command - minimal wrapper over fluentbase_build
#[derive(Parser)]
struct BuildCommand {
    /// Path to the contract directory
    path: PathBuf,

    /// Disable Docker (Docker is enabled by default)
    #[arg(long)]
    no_docker: bool,

    /// Cargo features to activate (comma-separated)
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    /// Do not activate default features
    #[arg(long)]
    default_features: bool,

    /// Additional artifacts to generate
    #[arg(short = 'g', long, value_enum, value_delimiter = ',')]
    generate: Vec<Artifact>,

    /// Output directory for artifacts
    #[arg(short, long)]
    output: Option<PathBuf>,
}

/// Verify command - builds locally and compares with deployed bytecode
#[derive(Parser)]
struct VerifyCommand {
    /// Path to the contract directory
    path: PathBuf,

    /// Disable Docker (Docker is enabled by default)
    #[arg(long)]
    no_docker: bool,

    /// Contract address to verify
    #[arg(long)]
    address: String,

    /// RPC endpoint URL
    #[arg(long)]
    rpc: String,

    /// Chain ID
    #[arg(long)]
    chain_id: u64,

    /// Cargo features to activate (comma-separated)
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    /// Do not activate default features
    #[arg(long)]
    no_default_features: bool,
}

/// Verification result for JSON output
#[derive(Serialize)]
struct VerificationResult {
    verified: bool,
    expected_hash: String,  // Hash from blockchain
    actual_hash: String,    // Hash from local build
    rustc_version: String,  // Affects bytecode
    sdk_version: String,    // Affects bytecode
    build_platform: String, // e.g., "docker:linux-x86_64"
}

/// RPC request structure
#[derive(Serialize)]
struct RpcRequest {
    jsonrpc: String,
    method: String,
    params: Vec<serde_json::Value>,
    id: u64,
}

/// RPC response structure
#[derive(serde::Deserialize)]
struct RpcResponse {
    result: Option<String>,
    error: Option<serde_json::Value>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build(cmd) => handle_build(cmd),
        Commands::Verify(cmd) => handle_verify(cmd),
    }
}

fn handle_build(cmd: BuildCommand) -> Result<()> {
    // Create BuildArgs with defaults and override with CLI options
    let mut build_args = BuildArgs::default();

    // Docker is enabled by default, disable if --no-docker is passed
    build_args.docker = !cmd.no_docker;

    // Set provided options
    build_args.features = cmd.features;
    build_args.no_default_features = !cmd.default_features;
    build_args.generate = cmd.generate;

    if let Some(output) = cmd.output {
        build_args.output = Some(output);
    }

    // Execute build
    let result = execute_build(&build_args, Some(cmd.path)).context("Failed to build contract")?;

    // Show what was built
    println!("Build completed successfully!");
    println!("WASM: {}", result.wasm_path.display());

    if let Some(rwasm) = &result.rwasm_path {
        println!("rWASM: {}", rwasm.display());
    }
    if let Some(wat) = &result.wat_path {
        println!("WAT: {}", wat.display());
    }
    if let Some(abi) = &result.abi_path {
        println!("ABI: {}", abi.display());
    }
    if let Some(solidity) = &result.solidity_path {
        println!("Solidity: {}", solidity.display());
    }
    if let Some(metadata) = &result.metadata_path {
        println!("Metadata: {}", metadata.display());
    }

    Ok(())
}

fn handle_verify(cmd: VerifyCommand) -> Result<()> {
    // Create BuildArgs with defaults for verification
    let mut build_args = BuildArgs::default();

    // Docker is enabled by default, disable if --no-docker is passed
    build_args.docker = !cmd.no_docker;

    // Set provided options
    build_args.features = cmd.features;
    build_args.no_default_features = cmd.no_default_features;

    // Always generate rwasm and metadata for verification
    build_args.generate = vec![Artifact::Rwasm, Artifact::Metadata];

    // Execute build
    let build_result = execute_build(&build_args, Some(cmd.path))
        .context("Failed to build contract for verification")?;

    // Get the rwasm path - required for verification
    let rwasm_path = build_result.rwasm_path.ok_or_else(|| {
        anyhow::anyhow!("rWASM artifact was not generated. This is required for verification.")
    })?;

    // Read the generated rwasm file
    let rwasm_bytes = fs::read(&rwasm_path).context("Failed to read generated rwasm file")?;

    // Calculate hash of locally built rwasm
    let local_hash = calculate_hash(&rwasm_bytes);

    // Fetch deployed bytecode from chain
    let deployed_bytecode = fetch_deployed_bytecode(&cmd.address, &cmd.rpc)
        .context("Failed to fetch deployed bytecode from chain")?;

    // Calculate hash of deployed rwasm
    let deployed_hash = calculate_hash(&deployed_bytecode);

    // Get metadata path - required for version info
    let metadata_path = build_result.metadata_path.ok_or_else(|| {
        anyhow::anyhow!("Metadata artifact was not generated. This is required for verification.")
    })?;

    // Read and parse metadata
    let metadata_content =
        fs::read_to_string(&metadata_path).context("Failed to read metadata file")?;
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_content).context("Failed to parse metadata")?;

    // Extract version information from metadata
    let rustc_version = metadata["environment"]["rustc_version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let sdk_version = format!(
        "{}-{}",
        metadata["environment"]["fluentbase_sdk"]["version"]
            .as_str()
            .unwrap_or("unknown"),
        metadata["environment"]["fluentbase_sdk"]["git_commit"]
            .as_str()
            .unwrap_or_default()
    );

    let build_platform = metadata["environment"]["build_platform"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    // Compare hashes
    let verified = local_hash == deployed_hash;

    // Create verification result
    let result = VerificationResult {
        verified,
        expected_hash: format!("0x{}", deployed_hash),
        actual_hash: format!("0x{}", local_hash),
        rustc_version,
        sdk_version,
        build_platform,
    };

    // Output JSON result
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

fn calculate_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn fetch_deployed_bytecode(address: &str, rpc: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::new();

    let request = RpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "eth_getCode".to_string(),
        params: vec![
            serde_json::Value::String(address.to_string()),
            serde_json::Value::String("latest".to_string()),
        ],
        id: 1,
    };

    let response = client
        .post(rpc)
        .json(&request)
        .send()
        .context("Failed to send RPC request")?;

    let rpc_response: RpcResponse = response.json().context("Failed to parse RPC response")?;

    if let Some(error) = rpc_response.error {
        anyhow::bail!("RPC error: {:?}", error);
    }

    let bytecode_hex = rpc_response
        .result
        .ok_or_else(|| anyhow::anyhow!("No result in RPC response"))?;

    // Remove 0x prefix if present and decode hex
    let bytecode_hex = bytecode_hex.strip_prefix("0x").unwrap_or(&bytecode_hex);
    let bytecode = hex::decode(bytecode_hex).context("Failed to decode bytecode hex")?;

    Ok(bytecode)
}
