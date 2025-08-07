# Fluentbase Framework

[![codecov](https://codecov.io/github/fluentlabs-xyz/fluentbase/graph/badge.svg?token=FCA43Y60LW)](https://codecov.io/github/fluentlabs-xyz/fluentbase)

**Fluentbase** is a modular framework providing an SDK and proving system for Fluent State Transition Functions (STFs).
It enables developers to build shared applications (smart contracts), dedicated applications, system precompiles, or
fully custom STFs.

> Fluentbase is an experimental project and a work in progress.
> All bindings, methods, and naming conventions within the codebase are subject to change and have not been
> standardized.
> The codebase is also not audited and not fully tested, which may lead to potential vulnerabilities or unexpected
> crashes.

---

## Modules Overview

### `bins`

Binary applications:

* `cli` — Command-line tool for building and verifying applications.

### `contracts`

Smart contract implementations:

* `examples` — A collection of example smart contracts used in E2E tests, benchmarks, and documentation.
* `genesis` — Precompiled genesis contracts (EVM, SVM, WASM runtimes).

### `crates`

Core libraries and infrastructure:

* `build` — Tools for compiling Rust applications into WASM binaries.
* `codec` — A custom ABI codec for encoding/decoding inputs, optimized for random-access reads. Inspired by Solidity's
  ABI, but uses a more WASM-friendly binary layout and alignment.
* `contracts` — Utilities for efficient parallel genesis builds.
* `erc20` — ERC20 contract utilities (marked for deprecation).
* `evm` — EVM execution support.
* `genesis` — Build scripts for generating Fluent L2 genesis files with precompiled contracts.
* `runtime` — rWASM/WebAssembly runtime enabling host function integrations.
* `sdk` — Developer SDK with all necessary types, macros, entrypoint definitions, and allocators.
* `svm` — SVM execution engine.
* `svm-shared` — Shared utilities for the SVM module.
* `testing` — Test harness for simulated STF execution contexts.
* `types` — Common primitive types shared across all crates.

### Other

* `docker` — Containerization helpers.
* `e2e` — End-to-end tests covering EVM transitions and WASM features.
* `revm` — Fork of the REVM crate, adapted for integration with the rWASM VM.

---

## Building & Testing

A `Makefile` in the root directory automates building all required modules and examples.

To build the entire project:

```bash
make
```

This will compile all contracts, examples, and generate the genesis files.

---

## Examples

Fluentbase supports building various types of applications using a unified interface.
The simplest application can be found in the `examples` directory, along with more advanced use cases.

---

## Supported Languages

Fluentbase SDK currently supports developing smart contracts in:

* Rust
* Solidity
* Vyper

---

## Fluentbase Execution

Fluentbase runs on the **rWasm VM** (reduced WebAssembly), a Fluent-specific execution environment.

* Fully compatible with standard WebAssembly binary format.
* Optimized for Zero-Knowledge (ZK) proof generation.
* Uses a reduced instruction set.
* Embeds metadata sections directly in the binary for more efficient proving.

You can find more rWasm related info here: https://github.com/fluentlabs-xyz/rwasm

---

## Contributing

We welcome community contributions!
Please refer to the [Contributing Guidelines](CONTRIBUTING.md) for details on how to get involved.