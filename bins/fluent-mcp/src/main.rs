//! Read-only stdio MCP adapter for Fluent developer context.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "fluent-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
#[command(about = "Run a read-only Fluent MCP adapter over stdio")]
struct Args {
    /// Fluentbase repository root used for curated docs/resources.
    #[arg(long, env = "FLUENT_MCP_REPO_ROOT", default_value = ".")]
    repo_root: PathBuf,

    /// Allow an RPC URL for node status calls. Localhost RPC is always allowed.
    #[arg(
        long = "allow-rpc",
        env = "FLUENT_MCP_ALLOW_RPC",
        value_delimiter = ','
    )]
    allow_rpc: Vec<String>,

    /// Default RPC URL used when fluent_node_status is called without rpcUrl.
    #[arg(long, env = "FLUENT_MCP_DEFAULT_RPC")]
    default_rpc: Option<String>,
}

#[derive(Debug)]
struct Server {
    repo_root: PathBuf,
    allowed_rpc: Vec<String>,
    default_rpc: Option<String>,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct Request {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceReadParams {
    uri: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptGetParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeStatusArgs {
    #[serde(default)]
    rpc_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChainInfoArgs {
    #[serde(default)]
    chain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonRpcError {
    code: i64,
    message: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let repo_root = args
        .repo_root
        .canonicalize()
        .with_context(|| format!("invalid repo root {}", args.repo_root.display()))?;

    let server = Server {
        repo_root,
        allowed_rpc: args.allow_rpc,
        default_rpc: args.default_rpc,
        client: Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("failed to initialize HTTP client")?,
    };

    server.run(io::stdin().lock(), io::stdout().lock())
}

impl Server {
    fn run<R: Read, W: Write>(&self, input: R, mut output: W) -> Result<()> {
        let mut reader = BufReader::new(input);
        while let Some(message) = read_message(&mut reader)? {
            let request: Request = match serde_json::from_str(&message) {
                Ok(request) => request,
                Err(err) => {
                    write_message(
                        &mut output,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": null,
                            "error": {"code": -32700, "message": err.to_string()}
                        }),
                    )?;
                    continue;
                }
            };

            if request.id.is_none() {
                continue;
            }

            let id = request.id.clone().unwrap_or(Value::Null);
            let response = match self.handle(request) {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(err) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": JsonRpcError {
                        code: -32603,
                        message: err.to_string(),
                    }
                }),
            };
            write_message(&mut output, &response)?;
        }
        Ok(())
    }

    fn handle(&self, request: Request) -> Result<Value> {
        if request.jsonrpc.as_deref() != Some("2.0") {
            bail!("expected JSON-RPC 2.0 request");
        }

        match request.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION,
                },
                "capabilities": {
                    "resources": {},
                    "tools": {},
                    "prompts": {},
                }
            })),
            "ping" => Ok(json!({})),
            "resources/list" => self.list_resources(),
            "resources/read" => {
                let params: ResourceReadParams = serde_json::from_value(request.params)?;
                self.read_resource(&params.uri)
            }
            "tools/list" => self.list_tools(),
            "tools/call" => {
                let params: ToolCallParams = serde_json::from_value(request.params)?;
                self.call_tool(&params.name, params.arguments)
            }
            "prompts/list" => self.list_prompts(),
            "prompts/get" => {
                let params: PromptGetParams = serde_json::from_value(request.params)?;
                self.get_prompt(&params.name, params.arguments)
            }
            method => Err(anyhow!("unsupported method {method}")),
        }
    }

    fn list_resources(&self) -> Result<Value> {
        let mut resources = vec![
            resource(
                "fluent://docs/index",
                "Fluent docs index",
                "Curated local docs available through the adapter",
            ),
            resource(
                "fluent://chains/dev",
                "Fluent dev chain",
                "Local development chain metadata",
            ),
            resource(
                "fluent://chains/fluent-devnet",
                "Fluent devnet",
                "Fluent Devnet chain metadata",
            ),
            resource(
                "fluent://chains/fluent-testnet",
                "Fluent testnet",
                "Fluent Testnet chain metadata",
            ),
            resource(
                "fluent://chains/fluent-mainnet",
                "Fluent mainnet",
                "Fluent Mainnet chain metadata",
            ),
            resource(
                "fluent://artifacts/runtime-genesis",
                "Runtime and genesis artifacts",
                "Read-only runtime/genesis artifact overview",
            ),
        ];

        for doc in curated_docs(&self.repo_root)? {
            let name = doc
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow!("invalid doc file name"))?;
            resources.push(resource(
                &format!("fluent://docs/{name}"),
                name,
                "Curated Fluentbase documentation",
            ));
        }

        Ok(json!({ "resources": resources }))
    }

    fn read_resource(&self, uri: &str) -> Result<Value> {
        let text = match uri {
            "fluent://docs/index" => self.docs_index()?,
            "fluent://chains/dev" => chain_info("dev")?,
            "fluent://chains/fluent-devnet" => chain_info("fluent-devnet")?,
            "fluent://chains/fluent-testnet" => chain_info("fluent-testnet")?,
            "fluent://chains/fluent-mainnet" => chain_info("fluent-mainnet")?,
            "fluent://artifacts/runtime-genesis" => self.artifact_info()?,
            _ if uri.starts_with("fluent://docs/") => {
                let name = uri.trim_start_matches("fluent://docs/");
                self.read_curated_doc(name)?
            }
            _ => bail!("unknown resource URI {uri}"),
        };

        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "text/markdown",
                "text": text,
            }]
        }))
    }

    fn list_tools(&self) -> Result<Value> {
        Ok(json!({
            "tools": [
                {
                    "name": "fluent_node_status",
                    "description": "Read sanitized status from an allowlisted Fluent/Ethereum JSON-RPC endpoint.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "rpcUrl": {
                                "type": "string",
                                "description": "Optional RPC URL. Localhost is allowed by default; other URLs must be passed with --allow-rpc."
                            }
                        },
                        "additionalProperties": false
                    }
                },
                {
                    "name": "fluent_chain_info",
                    "description": "Return static chain metadata for dev, fluent-devnet, fluent-testnet, or fluent-mainnet.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "chain": {
                                "type": "string",
                                "enum": ["dev", "fluent-devnet", "fluent-testnet", "fluent-mainnet"]
                            }
                        },
                        "additionalProperties": false
                    }
                }
            ]
        }))
    }

    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let text = match name {
            "fluent_node_status" => {
                let args: NodeStatusArgs = serde_json::from_value(arguments)?;
                self.node_status(args)?
            }
            "fluent_chain_info" => {
                let args: ChainInfoArgs = serde_json::from_value(arguments)?;
                chain_info(args.chain.as_deref().unwrap_or("dev"))?
            }
            _ => bail!("unknown tool {name}"),
        };

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text,
            }]
        }))
    }

    fn list_prompts(&self) -> Result<Value> {
        Ok(json!({
            "prompts": [{
                "name": "debug_fluent_contract_failure",
                "description": "Guide a local Fluent contract/runtime failure investigation using sanitized context.",
                "arguments": [
                    {"name": "symptom", "description": "Failure symptom or error text", "required": true},
                    {"name": "chain", "description": "Fluent chain name, if known", "required": false}
                ]
            }]
        }))
    }

    fn get_prompt(&self, name: &str, arguments: Value) -> Result<Value> {
        if name != "debug_fluent_contract_failure" {
            bail!("unknown prompt {name}");
        }

        let symptom = arguments
            .get("symptom")
            .and_then(Value::as_str)
            .unwrap_or("<describe the failure>");
        let chain = arguments
            .get("chain")
            .and_then(Value::as_str)
            .unwrap_or("dev or target Fluent network");

        Ok(json!({
            "description": "Debug a Fluent contract/runtime failure without exposing secrets.",
            "messages": [{
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Debug this Fluent contract/runtime failure on {chain}: {symptom}\n\nUse read-only context first: chain metadata, docs, sanitized RPC status, transaction/receipt data supplied by the user, and local build/test output. Do not request private keys, env vars, datadir contents, validator credentials, or internal topology. Ask for explicit confirmation before any write, deploy, or node-control action."
                    )
                }
            }]
        }))
    }

    fn docs_index(&self) -> Result<String> {
        let mut lines = vec!["# Fluentbase docs index".to_string(), String::new()];
        lines.push("- README.md".to_string());
        for doc in curated_docs(&self.repo_root)? {
            let rel = doc.strip_prefix(&self.repo_root).unwrap_or(&doc);
            lines.push(format!("- {}", rel.display()));
        }
        Ok(lines.join("\n"))
    }

    fn read_curated_doc(&self, name: &str) -> Result<String> {
        validate_doc_name(name)?;
        let candidates = [
            self.repo_root.join(name),
            self.repo_root.join("docs").join(name),
        ];
        for candidate in candidates {
            if candidate.is_file() && is_curated_doc(&self.repo_root, &candidate)? {
                return fs::read_to_string(&candidate)
                    .with_context(|| format!("failed to read {}", candidate.display()));
            }
        }
        bail!("doc {name} is not in the curated allowlist")
    }

    fn artifact_info(&self) -> Result<String> {
        let contracts = self.repo_root.join("contracts");
        let genesis = self.repo_root.join("crates/genesis");
        Ok(format!(
            "# Fluent runtime/genesis artifacts\n\n- contracts directory present: {}\n- genesis crate present: {}\n- policy: metadata only; generated artifacts and secrets are not exposed through this MCP adapter.\n",
            contracts.is_dir(),
            genesis.is_dir()
        ))
    }

    fn node_status(&self, args: NodeStatusArgs) -> Result<String> {
        let rpc_url = args
            .rpc_url
            .or_else(|| self.default_rpc.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8545".to_string());
        self.ensure_rpc_allowed(&rpc_url)?;

        let block_number = self.rpc_call(&rpc_url, "eth_blockNumber", json!([]))?;
        let chain_id = self.rpc_call(&rpc_url, "eth_chainId", json!([]))?;
        let client_version = self
            .rpc_call(&rpc_url, "web3_clientVersion", json!([]))
            .unwrap_or_else(|err| json!({"unavailable": err.to_string()}));

        Ok(serde_json::to_string_pretty(&json!({
            "rpcUrl": sanitize_rpc_url(&rpc_url),
            "chainId": chain_id,
            "blockNumber": block_number,
            "clientVersion": client_version,
        }))?)
    }

    fn rpc_call(&self, rpc_url: &str, method: &str, params: Value) -> Result<Value> {
        let response: Value = self
            .client
            .post(rpc_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            }))
            .send()
            .with_context(|| format!("RPC request {method} failed"))?
            .error_for_status()
            .with_context(|| format!("RPC request {method} returned an error status"))?
            .json()
            .with_context(|| format!("RPC response {method} was not JSON"))?;

        if let Some(error) = response.get("error") {
            bail!("RPC {method} returned error: {error}");
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("RPC {method} response did not include result"))
    }

    fn ensure_rpc_allowed(&self, rpc_url: &str) -> Result<()> {
        if is_local_rpc(rpc_url) || self.allowed_rpc.iter().any(|allowed| allowed == rpc_url) {
            return Ok(());
        }
        bail!("RPC URL is not allowlisted; use localhost or start with --allow-rpc")
    }
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<String>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>()?);
        }
    }

    let len = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    Ok(Some(String::from_utf8(body)?))
}

fn write_message<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn resource(uri: &str, name: &str, description: &str) -> Value {
    json!({
        "uri": uri,
        "name": name,
        "description": description,
        "mimeType": "text/markdown",
    })
}

fn curated_docs(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let mut docs = Vec::new();
    let readme = repo_root.join("README.md");
    if readme.is_file() {
        docs.push(readme);
    }
    let docs_dir = repo_root.join("docs");
    if docs_dir.is_dir() {
        for entry in fs::read_dir(docs_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                docs.push(path);
            }
        }
    }
    docs.sort();
    Ok(docs)
}

fn is_curated_doc(repo_root: &Path, path: &Path) -> Result<bool> {
    let path = path.canonicalize()?;
    let readme = repo_root.join("README.md").canonicalize().ok();
    if readme.as_ref() == Some(&path) {
        return Ok(true);
    }
    let docs_dir = repo_root.join("docs").canonicalize()?;
    Ok(path.starts_with(docs_dir) && path.extension().and_then(|ext| ext.to_str()) == Some("md"))
}

fn validate_doc_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("invalid doc name");
    }
    Ok(())
}

fn chain_info(chain: &str) -> Result<String> {
    let (name, chain_id, description) = match chain {
        "dev" => ("dev", "1337", "Local development chain"),
        "fluent-devnet" => ("fluent-devnet", "20993", "Fluent Devnet"),
        "fluent-testnet" => ("fluent-testnet", "20994", "Fluent Testnet"),
        "fluent-mainnet" => ("fluent-mainnet", "25363", "Fluent Mainnet"),
        _ => bail!("unknown chain {chain}"),
    };

    Ok(format!(
        "# {description}\n\n- name: `{name}`\n- chain id: `{chain_id}`\n- RPC policy: endpoints are user supplied and must be allowlisted for MCP node-status calls.\n"
    ))
}

fn is_local_rpc(rpc_url: &str) -> bool {
    rpc_url.starts_with("http://127.0.0.1:")
        || rpc_url.starts_with("http://localhost:")
        || rpc_url.starts_with("http://[::1]:")
}

fn sanitize_rpc_url(rpc_url: &str) -> String {
    let Some((scheme, rest)) = rpc_url.split_once("://") else {
        return "<invalid-url>".to_string();
    };
    if let Some(at_index) = rest.find('@') {
        format!("{scheme}://<redacted>@{}", &rest[at_index + 1..])
    } else {
        rpc_url.to_string()
    }
}
