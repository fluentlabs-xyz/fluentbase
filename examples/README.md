# Examples

========

In this repository, we provide examples of running apps on the Fluent network.
All these apps are developed using the Fluentbase SDK and can be proven with our circuits (coming soon).

To initialize, build, and deploy these examples, you can use the [gblend CLI](https://github.com/fluentlabs-xyz/gblend).

By the way, we also have a Makefile for building examples, so you can use it as well.

## Creating a new app

### From scratch

To create your own repository with example, create an empty crate and add fluentbase SDK dependency.

```bash
cargo new hello_world --lib
```

Now put the following code into `src/lib.rs` file.

```rust
#![no_std]
extern crate alloc;
// this line is required to enable Fluentbase panic handlers and allocators
extern crate fluentbase_sdk;

use fluentbase_sdk::{SysPlatformSDK, SDK};

#[no_mangle]
extern "C" fn deploy() {}

#[no_mangle]
extern "C" fn main() {
    let str = "Hello, World";
    SDK::sys_write(str.as_bytes());
}
```

As you can see, there are two functions that must be exported with exact names:

- `deploy` - this function is called before creating app (similar to Solidity's constructor)
- `main` - this one is getting called on each contract interaction

To add Fluentbase SDK dependency add the following dep in your `Cargo.toml` file:

```toml
[dependencies]
fluentbase-sdk = { git = "https://github.com/fluentlabs-xyz/fluentbase", default-features = false }
```

If you don't want to use EVM features then just disable `evm` feature flag.

Additionally add these lines into your `Cargo.toml` file:

```toml
[profile.release]
panic = "abort"
lto = true
opt-level = 'z'
strip = true
```

### Using templates (with gblend)

## Choose template

The `gblend init` command helps you bootstrap new projects using templates:

### List Templates

```bash
# List all available templates
gblend init rust -l
```

### Creating New Project

```bash
# Initialize project from template
gblend init rust -t greeting -p ./greeting
```

> [!NOTE]
> Replace `greeting` with any template name and `./greeting` with your desired project path.

### Post-Initialization Steps

After project creation, you'll want to:

1. Review the generated code in `lib.rs`
2. [Build your project](#build)
3. [Deploy your contract](#deploy)

> [!TIP]
> Templates provide a quick start with working examples and proper project structure. They're the recommended way to
> begin new Fluent Network projects.

## Build

The `gblend build` command compiles your smart contracts for deployment:

### Build Basic Usage

```bash
# Build project in release mode with .wat file generation
gblend build rust -r --wat
```

### Build Options

- Use `-r, --release` for optimized release builds
- Add `--wat` to generate WebAssembly text format
- Specify custom path with `-p, --path`

### Using Makefiles

The repository uses a two-level Makefile structure:

1. Root Makefile for building all examples:

```bash
# Build all examples
make all

# Build specific example
make greeting
make keccak256
```

2. Each example has its own Makefile with WASM compilation settings

### Project Structure

For simplicity, all examples are stored inside one crate and managed through Cargo features.

> [!NOTE]
> When adding new examples, remember to update both `Cargo.toml` and `Makefile` with your new features.

### Prerequisites

Install the required WebAssembly target:

```bash
rustup target add wasm32-unknown-unknown
```

> [!NOTE]
> Your compiled WASM binary will be located at `target/wasm32-unknown-unknown/release/<name>.wasm`

> [!TIP]
> Use `wasm2wat` tool to inspect the WebAssembly text format of your compiled binary:
>
> ```bash
> wasm2wat target/wasm32-unknown-unknown/release/hello_world.wasm
> ```

The next step is to [deploy your contract](#deploy).

## Deploy

The `gblend deploy` command provides several options for deploying your application:

### Network Selection

- Use `--local` for deploying to a local network
- Use `--dev` for deploying to the development network
- Specify a custom RPC endpoint with `--rpc <URL>` and `--chain-id <CHAIN_ID>` for other networks

### Authentication

You can provide your private key in several ways:

1. Command line: `--private-key <KEY>`
2. Environment variable: `DEPLOY_PRIVATE_KEY`
3. Environment file: Create `.env` file with `DEPLOY_PRIVATE_KEY=<your-key>`

### Deploy Basic Usage

```bash
# Deploy to local network
gblend deploy --local path/to/contract.wasm

# Deploy to devnet
gblend deploy --dev path/to/contract.wasm

# Deploy with custom RPC
gblend deploy --rpc https://your-node.network --chain-id 1234 path/to/contract.wasm
```

> [!TIP]
> Using environment files is recommended for managing private keys securely. Create a `.env` file in your project root:
>
> ```
> DEPLOY_PRIVATE_KEY=0x...
> ```

> [!NOTE]
> Additional parameters like `gas-limit`, `gas-price`, and `confirmations` can also be configured through environment
> variables or command line flags.
