# fluentbase-cli

The standard CLI tool for building and verifying Fluent smart contracts.

## Installation

```bash
# From repository
cargo install --path crates/cli

# Or from crates.io (when published)
cargo install fluentbase-cli
```

**Requirements:**

- Rust toolchain
- Docker (for reproducible builds)

## Commands

### Build

Build your Fluent smart contract with reproducible builds using Docker:

```bash
# Build with default settings (Docker enabled)
fluentbase-cli build .

# Generate specific artifacts
fluentbase-cli build . --generate rwasm,abi,metadata

# Build with custom features
fluentbase-cli build . --features my-feature,another-feature

# Build without Docker (not recommended for production)
fluentbase-cli build . --no-docker
```

**Options:**

- `--generate` - Specify artifacts to generate (comma-separated)
- `--features` - Enable Cargo features (comma-separated)
- `--default-features` - Enables default Cargo features
- `--output` - Custom output directory (default: `./out`)
- `--no-docker` - Disable Docker (builds may not be reproducible)

### Verify

Verify that a deployed contract matches your source code:

```bash
fluentbase-cli verify . \
  --address 0x1234567890abcdef1234567890abcdef12345678 \
  --rpc https://rpc.fluent.xyz \
  --chain-id 1337
```

**Required parameters:**

- `--address` - The deployed contract address
- `--rpc` - RPC endpoint URL
- `--chain-id` - Chain ID of the network

**Optional parameters:**

- `--features` - Cargo features (must match deployment)
- `--no-default-features` - Disable default features (must match deployment)

**Output:** JSON with verification result:

```json
{
  "verified": true,
  "expected_hash": "0x...",
  "actual_hash": "0x...",
  "rustc_version": "rustc 1.87.0 (abc123)",
  "sdk_version": "0.2.1-dev-9bf1ee3d",
  "build_platform": "docker:linux-x86_64"
}
```

## Available Artifacts

- `rwasm` - Reduced WebAssembly (deployed bytecode)
- `wasm` - Raw WebAssembly bytecode
- `abi` - Contract ABI in JSON format
- `metadata` - Build metadata for verification
- `wat` - WebAssembly Text format
- `solidity` - Solidity interface

## Why Docker?

Docker ensures that contracts built on different machines produce identical bytecode. This is crucial for:

- Verification services
- Team collaboration
- Reproducible deployments

## Examples

### Building for production

```bash
# Always use Docker for production builds
fluentbase-cli build . --generate rwasm,abi,metadata
```

### Verifying a mainnet contract

```bash
fluentbase-cli verify . \
  --address 0xYourContractAddress \
  --rpc https://mainnet.fluent.xyz \
  --chain-id 9999 \
  --features mainnet
```

### Development build (faster, no Docker)

```bash
# Only for local testing
fluentbase-cli build . --no-docker --generate rwasm
```

## Notes

- Verification rebuilds your contract locally and compares it with the deployed bytecode
- Always use the same features and build configuration for verification as used during deployment
- The CLI uses `fluentbase-build` under the hood, ensuring consistency across all tools
- The verify command runs in a controlled environment and doesn't use Docker internally

## License

See LICENSE in the repository root.
