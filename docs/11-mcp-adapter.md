# Fluent MCP Adapter

`fluent-mcp` is an opt-in Model Context Protocol server for local developer and
agent workflows. It runs outside the Fluent node, runtime, STF, and consensus
paths. The adapter exposes curated read-only context over stdio so MCP clients
can inspect Fluent docs, chain metadata, and sanitized local node status without
changing chain behavior.

## Scope

The first version intentionally stays small:

- read-only resources for selected local docs, chain metadata, and
  runtime/genesis artifact metadata
- `fluent_chain_info` for static chain identifiers
- `fluent_node_status` for sanitized JSON-RPC status from localhost or an
  explicitly allowlisted RPC URL
- `debug_fluent_contract_failure` as a local debugging prompt

No deploy, transaction signing, filesystem write, node-control, runtime upgrade,
or secret-reading tools are exposed.

## Running

From a Fluentbase checkout:

```bash
cargo run -p fluent-mcp -- --repo-root .
```

By default, `fluent_node_status` may query only localhost RPC URLs such as
`http://127.0.0.1:8545`. To allow another endpoint, pass it explicitly:

```bash
cargo run -p fluent-mcp -- --allow-rpc https://rpc.example.invalid
```

Multiple endpoints can be supplied by repeating `--allow-rpc` or by using the
comma-separated `FLUENT_MCP_ALLOW_RPC` environment variable.

## Security Model

The adapter treats MCP as developer/operator I/O rather than chain logic:

- keep it as an external process, not a production-node default
- bind clients through stdio first
- use curated resource roots only
- allow remote RPC calls only when configured explicitly
- redact credentials embedded in RPC URLs before returning tool output
- never expose env vars, private keys, datadir contents, validator/sequencer
  credentials, raw logs with secrets, or internal topology

Future Streamable HTTP support should add Origin validation, authentication, and
resource metadata before any non-local exposure.
