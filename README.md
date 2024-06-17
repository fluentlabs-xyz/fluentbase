# Introduction to Fluentbase

Fluentbase is a framework that introduces an SDK and a proving system for Fluent State Transition Functions (STF). The
framework can be used by developers to create shared applications (aka smart contracts), dedicated applications, system
precompile contracts or just for custom STFs.

## Don't use on production

Fluentbase is an experimental development that is still work in progress. All bindings, methods and namings inside
codebase are not standardized and can be changes significantly. Also the codebase is not audited and not fully tested,
that can cause potential vulnerabilities or crashes.

## Modules

* `bin` - a crate with a binary application that is used for translating WASM applications to rWASM. It’s required only
  for creating system precompiled contracts where direct translation from WASM to rWASM is required.
* `crates` - folder with all Fluentbase modules
    * `codec` - a crate with a custom ABI codec for encoding/decoding input messages. This codec is optimized for random
      reads that are used to extract only required information from passed system context. It’s very similar to Solidity
      ABI encoding, but uses more WASM friendly binary encoding and alignment.* `contracts` - a crate with all system
      precompiled contracts that brings
      support of different EEs compatibility,
      including EVM, SVM, WASM and all
      corresponding system contracts like
      blake2, sha256, etc.
    * `core` - a core of EE runtimes with EVM, SVM, WASM support including deployment logic, AOT translation and
      contract execution.
    * `evm` (outdated) - repository with EVM AOT compiler.
    * `genesis` - a program for creating genesis files for Fluent L2 network with precompiled system and compatibility
      contracts.
    * `poseidon` - library for poseidon hashing.
    * `revm` (migrating) - a fork of revm crate, but optimized and adapted for Fluentbase SDK methods and maps original
      revm’s database objects into Fluentbase’s structures. It’s needed to execute evm transactions inside reth.
    * `runtime` - a basic execution runtime of rWASM that enables Fluentbase’s host functions.
    * `sdk` - a basic repository for developers where they can include all required types and methods to develop their
      applications. It also includes all macros, definition of entrypoint, allocator, etc.
    * `types` - basic primitive types for all crates inside this repository.
    * `zktrie` - implementation of zktrie (sparse merkle binary trie).
* `e2e` (partially outdated) — a set of e2e tests for testing EVM transition and other WASM features.
* `examples` - a folder with examples that can be built using Fluentbase SDK

## Build and testing

To build Fluentbase, there is a Makefile in the root folder that builds all required dependencies and examples.
You can run `make` command to build all contracts, examples and genesis files.

Resulting files can be found in the following directories:

* `crates/contracts/assets` - wasm and rwasm binaries for all precompiles, system contracts and compatability contracts.
* `crates/genesis/assets` - reth/geth compatible genesis files with injected rwasm binaries (is used by reth).
* `examples/*` - each folder contains `lib.wasm` and `lib.wat` files that matches compiled example bytecode.

Testing includes all EVM official testing suite. This test consumes a lot of resources. We also suggest to increase Rust
stack size to 20 mB.

```bash=
RUST_MIN_STACK=20000000 cargo test --no-fail-fast
```

P.S: Some tests are still failing (like zktrie), but 99% of them pass.

## Examples

Fluentbase can be used to develop different types of applications, in most of the cases the same interface is used. Here
is the simplest application can be developed using Fluentbase.

```rust=
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, SharedAPI};

#[derive(Default)]
struct GREETING;

impl GREETING {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn main<SDK: SharedAPI>(&self) {
        // write "Hello, World" message into output
        SDK::write("Hello, World".as_bytes());
    }
}

basic_entrypoint!(GREETING);
```

## Supported Languages

Fluentbase SDK currently supports writing smart contracts in:

* Rust
* Solidity
* Vyper

## Fluentbase Operation

Fluentbase operates using the Fluent's rWasm VM (reduced WebAssembly).
This VM uses 100% compatible WebAssembly
binary representation, optimized for Zero-Knowledge (ZK) operations.
The instruction set is reduced, and sections are
embedded inside the binary to simplify the proving process.

## Limitations and Future Enhancements

As of now, Fluentbase does not support floating-point operations. However, this feature is on the roadmap for future
enhancements.


