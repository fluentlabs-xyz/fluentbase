# Examples

Example contracts written with the Fluentbase SDK. Each example is its own crate in a small Cargo workspace that is
separate from the root workspace. Every crate compiles to a WASM contract for the Fluent network and also builds as a
normal Rust crate on the host, so its unit tests run without a node.

The root build embeds the examples as well: the build script of `crates/contracts` compiles every member of this
workspace to `wasm32-unknown-unknown` and exposes the bytes as `fluentbase_contracts::FLUENTBASE_EXAMPLES_<NAME>`,
which the runtime tests and benchmarks in `e2e/runtime` execute.

---

## Layout

- `greeting` — the minimal contract: `entrypoint!` with a single `main_entry` that writes to the output.
- `abi-solidity` — decoding and encoding Solidity ABI values by hand with `SolidityABI`.
- `balance` — reading an account balance through `SharedAPI`.
- `checkmate` — using an external `no_std` crate (the `shakmaty` chess engine) inside a contract.
- `client-solidity` — `#[client(mode = "solidity")]`: typed clients for cross-contract calls.
- `constructor-params` — a `deploy` entry that reads constructor input and persists it in storage.
- `erc20` — a complete ERC-20 token: `#[router]`, `#[derive(Event)]`, `#[constructor]` and typed storage.
- `json` — parsing JSON input with `serde_json_core`.
- `keccak`, `sha256` — hashing through the SDK's crypto syscalls.
- `memory-oom` — allocating the maximum allowed memory; exercises the runtime's out-of-memory path.
- `panic` — what a `panic!` inside a contract looks like to the caller.
- `router-solidity` — `#[router(mode = "solidity")]` with explicit `#[function_id]` selectors and ABI validation.
- `rwasm` — compiling a WASM module into rWasm from inside a contract.
- `secp256k1` — signature verification with the `libsecp256k1` crate.
- `simple-storage` — storage reads and writes with `solidity_storage!`.
- `storage` — an ERC-20 built on `solidity_storage!`; the source is currently commented out.
- `storage-usage` — typed storage: `StorageMap`, `StorageVec`, `StorageString`, nested maps and custom slots.
- `tiny-keccak` — running a third-party hashing crate. Prefer the SDK's `crypto_keccak256` in real contracts.
- `unwiped-output` — output written before a later syscall is kept.
- `svm` — Solana programs for the SVM runtime. Excluded from the workspace while SVM is unstable (see the root
  README).

---

## Building

### Prerequisites

```bash
rustup target add wasm32-unknown-unknown
```

The toolchain version is pinned in the root `rust-toolchain.toml`.

### Build an example

From this directory:

```bash
cargo build -p fluentbase-examples-greeting --release --target wasm32-unknown-unknown --no-default-features
```

The artifact lands in `target/contracts/wasm32-unknown-unknown/release/fluentbase_examples_greeting.wasm` at the
repository root (`.cargo/config.toml` points the target directory there). `--no-default-features` switches off the
`std` feature, which exists only for host-side tests.

`make build` at the repository root compiles every example the same way as part of the contracts build.

> Tip: `wasm2wat` from [wabt](https://github.com/WebAssembly/wabt) prints the text form of a compiled contract, which
> is a quick way to check its imports and exports:
>
> ```bash
> wasm2wat target/contracts/wasm32-unknown-unknown/release/fluentbase_examples_greeting.wasm
> ```

---

## Testing

Unit tests run on the host against `fluentbase_testing::TestingContextImpl`, an in-memory implementation of the SDK:

```bash
cargo test -p fluentbase-examples-greeting # one example
cargo test --workspace                     # all examples
```

`make test` at the repository root runs the same suites with `cargo nextest` in release mode, and `make clippy` lints
this workspace with `-D warnings`; CI does both.

---

## Creating a new app

### Cargo.toml

Create a library crate and depend on the published SDK; that is all the contract itself needs. Unit tests additionally
need `fluentbase-testing`, which is not on crates.io, so pull it from the repository. It brings the SDK and the runtime
from the same checkout, so patch the registry SDK to that source as well and only one copy of the crate ends up in the
build:

```toml
[package]
name = "hello-world"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
fluentbase-sdk = { version = "1.4.2", default-features = false }

[dev-dependencies]
fluentbase-testing = { git = "https://github.com/fluentlabs-xyz/fluentbase", branch = "devel" }

[features]
default = ["std"]
std = ["fluentbase-sdk/std", "fluentbase-testing/std"]

[profile.release]
panic = "abort"
lto = "fat"
opt-level = 3
strip = true
codegen-units = 1

[patch.crates-io]
fluentbase-sdk = { git = "https://github.com/fluentlabs-xyz/fluentbase", branch = "devel" }
```

> Note: use the default branch rather than a release tag for the git dependencies. The workspace references the
> `revm-rwasm` fork by branch, so an older tag can fall out of step with it; `v1.4.2` already no longer builds on the
> host against the current fork.

### Function entrypoint

The simplest contract is a function that receives the SDK and writes to the output. `entrypoint!` expands into the
`main` and `deploy` exports the runtime calls, plus the panic handler and allocator a `no_std` WASM binary needs:

```rust
#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    sdk.write("Hello, World".as_bytes());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_testing::TestingContextImpl;

    #[test]
    fn test_contract_works() {
        let sdk = TestingContextImpl::default();
        main_entry(sdk.clone());
        assert_eq!(&sdk.take_output(), "Hello, World".as_bytes());
    }
}
```

- `main` runs on every call to the contract.
- `deploy` runs once when the contract is created, like a Solidity constructor. Pass a second function to
  `entrypoint!(main_entry, deploy_entry)` to handle it; see `constructor-params`.

### Solidity-compatible router

For a contract with several methods, describe them as a trait and let `#[router(mode = "solidity")]` generate the
selector dispatch and ABI codec. `#[derive(Contract)]` provides the `new` constructor and `basic_entrypoint!` wires
`deploy` and `main` to the struct:

```rust
#![cfg_attr(not(feature = "std"), no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    SharedAPI,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: String) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&self, message: String) -> String {
        message
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(App);
```

`router-solidity` shows the full version with selector validation and tests that compare the generated calldata
against `alloy-sol-types`, and `erc20` adds events, a constructor and typed storage on top.

### Build and test

```bash
cargo test
cargo build --release --target wasm32-unknown-unknown --no-default-features
```

The contract is at `target/wasm32-unknown-unknown/release/hello_world.wasm`.

---

## Deploying

[gblend](https://github.com/fluentlabs-xyz/gblend) is the CLI for Fluent applications. It is a fork of Foundry's
`forge` that understands WASM contracts, so it scaffolds, builds, deploys and verifies both Rust and Solidity code:

```bash
curl -sSL https://raw.githubusercontent.com/fluentlabs-xyz/gblend/refs/tags/latest/gblendup/install | bash
gblendup

gblend init my-project          # scaffold a blended Rust + Solidity project
gblend build                    # compile everything in it
gblend create my_contract.wasm --wasm --rpc-url https://rpc.testnet.fluent.xyz --private-key $PRIVATE_KEY --broadcast
```

The full set of commands, templates and verification flags is documented at https://docs.fluent.xyz/gblend/usage.
