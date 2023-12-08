Examples
========

In this repository we provide examples for running apps in Fluent network.
All these apps are developed using Fluentbase SDK and can be proven using our circuits (coming soon).

In the make file you can find commands for building apps.
We require to install `wasm32-unknown-unknown` compilation target, WASI support is coming later.

```bash
rustup target add wasm32-unknown-unknown
```

## Use one of 

To compile examples (btw compiled artifacts are presented in the bin folder already) just run next command:
You also can compile specific test by passing it to the make command:

```bash
# to compile everything
make all
# to compile specific example
make greeting
make keccak256
make storage
```

For the simplicity we store all apps inside one crate and manage its compilation using features.
If you want to add new example into repo then don't forget to modify `Cargo.toml` and `Makefile` with you new feature.

But we suggest to create new crate.

## Creating new app

To create your own repository with example just create an empty crate and add fluentbase SDK dependency.

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

As you can see there two functions that must be exported with exact names:
- `deploy` - this function is called before creating app (similar to Solidity's constructor)
- `main` - this one is getting called on each contract interaction

To add Fluentbase SDK dependency add the following dep in your `Cargo.toml` file:

```toml
[dependencies]
fluentbase-sdk = { path = "https://github.com/fluentlabs-xyz/fluentbase", default-features = false, features = ["evm"] }
```

If you don't want to use EVM features then just disable `evm` feature flag.

Additionally add these lines into your `Cargo.toml` file:

```toml
[lib]
crate-type = ["cdylib"]

[profile.release]
panic = "abort"
lto = true
opt-level = 'z'
strip = true
```

You compiled WASM binary is located here `target/wasm32-unknown-unknown/release/hello_world.wasm`.

You can convert your WASM binary into WAT format to see textual representation.

```bash
wasm2wat target/wasm32-unknown-unknown/release/hello_world.wasm
```

## Deploy your app to the Fluent

We provide a JS script for deploying apps, so before running it you must install required dependencies.
Also make sure you have NodeJS installed.

```bash
yarn
```

Now you can call `deploy-contract.js` script to deploy your just compiled app.

```bash
node deploy-contract.js --dev target/wasm32-unknown-unknown/release/hello_world.wasm
```

Flag `--dev` here means dev network.
You can use `--local` if you're running and testing Fluent locally.

You can also provide a private key using `DEPLOYER_PRIVATE_KEY` env, but there is hardcoded one inside the script.

In the script output you can see transaction and receipt info, contract address and output message.
In our case we can see `Hello, World` message.
This script invokes contract after deployment with empty parameters.

For example, we can also call one of existing apps, lets say `keccak256` using the following command:

```bash
node deploy-contract.js --dev ./bin/keccak256.wasm
```

It returns next message `0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470` that is equal to empty Keccak256 hash.

## Codecs and contexts

We haven't standardized code yet, so we can't pass block/tx context elements inside app right now.
But we're going to bring it to the next devnet version.