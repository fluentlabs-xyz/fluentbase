# Fluentbase Contracts

This directory is the Cargo workspace for the system contracts that form the Fluent genesis file and ship in runtime
upgrades: the EVM, WASM and token runtimes, and the privileged contracts that manage a live chain. Every crate is written with the Fluentbase SDK, compiles to `wasm32-unknown-unknown`, and also builds as a
normal Rust crate on the host so that its unit tests run without a node.

The workspace is separate from the root workspace, but the root build drives it: the build script of `crates/contracts`
compiles every member here to WASM and embeds the artifacts as `fluentbase_contracts::FLUENTBASE_CONTRACTS_<NAME>`,
and `crates/genesis` maps those artifacts to their reserved addresses. A plain `cargo build` at the repository root
therefore rebuilds everything in this directory.

---

## Crates

### Runtimes

- `evm` — the EVM runtime. Executes EVM bytecode for every account that delegates to `PRECOMPILE_EVM_RUNTIME`. The
  interpreter is pinned to a single hardfork (Osaka) and is replaced through `runtime-upgrade`, so EVM semantics are
  upgraded forklessly.
- `wasm` — the WASM runtime. Its deploy path compiles the submitted WASM module into rWasm under engine fuel
  metering and rejects results larger than `RWASM_MAX_CODE_SIZE`.
- `universal-token` — an ERC-20-style token runtime with a 4-byte selector ABI, ERC-2612 permits and optional
  mintable/pausable plugins.

### Ethereum precompiles

There are no crates for the Ethereum precompiles (`0x01` to `0x11`, and `0x100` for EIP-7951): the node executes
revm's native implementations for those addresses (`RwasmPrecompiles` in `crates/revm`), so a fresh genesis installs
no code there. The live networks were launched with rWASM guests at those addresses; that bytecode remains in their
state, is shadowed by the native provider and is never executed, and a runtime upgrade of one of those addresses does
not change execution. `e2e/runtime/src/precompile_vectors.rs` pins that a transaction to each address agrees with
the bare `revm-precompile` function on output and gas; parity with the guests was established by running that
suite once against them before their removal.

### System and utility contracts

- `eip2935` — the EIP-2935 ring buffer of recent block hashes.
- `fee-manager` — collects protocol fees. The owner can withdraw the balance, transfer ownership or renounce it.
- `runtime-upgrade` — privileged contract that replaces runtime and system contract bytecode on a live chain
  (`upgradeTo`, `planUpgrade` and friends). Its README documents the authorization model.
- `nitro` — AWS Nitro Enclaves attestation document verifier.
- `webauthn` — WebAuthn assertion verification over P-256.
- `create2-factory` — the deterministic deployment proxy. Not a Rust crate: it holds the Yul source and the EVM
  bytecode that genesis places at the canonical factory address. Excluded from the workspace.

Every Rust crate ships its own README and an `abi.json`.

---

## Building

The recommended path is the root build, which compiles all contracts to WASM, converts them to rWasm and embeds them:

```bash
make build # or: cargo build
```

To build a single contract by hand from this directory:

```bash
cargo build -p fluentbase-contracts-eip2935 --release --target wasm32-unknown-unknown --no-default-features
```

The artifact lands in `target/contracts/wasm32-unknown-unknown/release/fluentbase_contracts_eip2935.wasm` at the
repository root. `.cargo/config.toml` sets that target directory and the rustflags every contract is built with
(1 MiB stack, bulk memory, tail calls), and `--no-default-features` switches off the `std` feature that only exists
for host-side tests.

Reproducible builds run the WASM compilation inside the pinned `fluentbase-build` Docker image, which is what the
release workflow does:

```bash
FLUENTBASE_CONTRACTS_DOCKER=true cargo build
```

`FLUENTBASE_BUILD_DOCKER_IMAGE`, `FLUENTBASE_BUILD_DOCKER_TAG` and `FLUENTBASE_BUILD_DOCKER_DIGEST` override the
image, and `FLUENTBASE_CONTRACTS_IGNORE_DEFAULT_RUST_FLAGS` controls whether `fluentbase-build` injects its own
rustflags on top of `.cargo/config.toml`. `make wasm_contracts_sizes` prints the size of every compiled contract.

### Prerequisites

```bash
rustup target add wasm32-unknown-unknown
```

The toolchain version is pinned in the root `rust-toolchain.toml`.

---

## Testing

Unit tests run on the host with the `std` feature enabled (the default):

```bash
cargo test -p fluentbase-contracts-eip2935 # one crate
cargo test --workspace                    # all crates
```

`make test` at the repository root runs the same suites with `cargo nextest` in release mode, and `make clippy`
lints this workspace with `-D warnings`; CI does both.

---

## Workspace configuration

`Cargo.toml` in this directory owns the shared configuration:

- **Members** — every subdirectory except `.cargo`, `target`, `out` (output of the `fluentbase-build` CLI)
  and `create2-factory`.
- **Fluentbase crates** — `fluentbase-build`, `fluentbase-sdk`, `fluentbase-evm` and `fluentbase-testing`, all as
  path dependencies into `../crates`, with `[patch.crates-io]` entries so that transitive dependencies resolve to
  the same local copies.
- **revm** — `revm-precompile` from the `fluentlabs-xyz/revm-rwasm` fork (`v107-patched` branch).
- **Misc** — `alloy-sol-types`, `hex`, `k256`, `hashbrown`, `spin`, and `ecdsa` from the `rwasm-patches/signatures`
  fork. `tiny-keccak` is patched to its `rwasm` branch.
- **Features** — each crate exposes `default = ["std"]` for host tests and `debug-print` for runtime tracing. `evm`
  additionally has `permissive-contract-size`, which the root build uses for a second, size-unlimited artifact.
- **Profiles** — `release` is `lto = "fat"`, `opt-level = 3`, `panic = "abort"`, `codegen-units = 1`, stripped;
  `dev` uses `opt-level = "z"` without debug info so that debug WASM binaries stay small.

---

## Notes

- Every crate starts with `#![cfg_attr(target_arch = "wasm32", no_std, no_main)]`: `no_std` on WASM, `std` on the
  host.
- Runtimes and system contracts use `system_entrypoint!` with a
  `main_entry(sdk: &mut impl SystemAPI) -> Result<(), ExitCode>` (and an optional `deploy_entry`). Router-style
  contracts such as `fee-manager` and `runtime-upgrade` use
  `#[derive(Contract)]`, `#[router(mode = "solidity")]` and `basic_entrypoint!`, the same API as the
  [examples](../examples/README.md).
- Addresses live in `crates/types` as `PRECOMPILE_*` constants, including the canonical Ethereum precompile
  addresses (`0x01` to `0x11`, and `0x100` for EIP-7951) that the node serves natively. `eip2935` sits at the address
  the EIP specifies, and the Fluent runtimes and system contracts occupy the reserved `0x…5200xx` range. The same
  module lists which addresses the system runtime executes and which are metered by the engine.

## Repository

https://github.com/fluentlabs-xyz/fluentbase
