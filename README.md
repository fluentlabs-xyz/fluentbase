# Fluentbase Framework

[![codecov](https://codecov.io/github/fluentlabs-xyz/fluentbase/graph/badge.svg?token=FCA43Y60LW)](https://codecov.io/github/fluentlabs-xyz/fluentbase)
[![Test](https://github.com/fluentlabs-xyz/fluentbase/actions/workflows/ci.yml/badge.svg)](https://github.com/fluentlabs-xyz/fluentbase/actions/workflows/ci.yml)

**Fluentbase** is a modular framework for building execution environments and smart contracts that compile into **rWasm
IR** and run in a unified, proof-friendly runtime.

---

## Fluentbase + rWasm (TL;DR)

Fluentbase uses **Blended Execution**: EVM/SVM/WASM are treated as compatibility layers, while execution converges to a
single intermediate representation (**rWasm VM**) and one provable state transition function (STF).

> Note: SVM is still under development and remains in an extremely unstable state. That’s why it has been removed from
> the genesis files and hidden behind a feature flag.

### Architecture

Traditional multi-VM architecture:

```text
EVM | SVM | WASM | UST (adapters)
   \    |    /     /
distinct execution semantics
 + distinct proving surfaces
```

Fluentbase blended-VM architecture:

```text
EVM / SVM / WASM / UST (runtimes)
          ↓
   System contracts
          ↓
      rWasm IR
          ↓
 Unified execution VM
          ↓
    ZK proof system
```

By leveraging this concept, the Fluentbase runtime supports multiple execution environments within a single account
space.
As a result, developers can deploy applications from EVM, SVM, WASM, and others, while enjoying seamless cross-state
interaction.

### Quick comparison

| Property                | Fluentbase                        |
|-------------------------|-----------------------------------|
| Execution model         | Unified rWasm IR                  |
| VM count                | 1 (rWasm VM)                      |
| Environment support     | EVM / SVM / WASM / UST            |
| Proof target            | Single STF (rWasm IR)             |
| Determinism             | Strong                            |
| ZK efficiency           | First-class constraint            |
| Cross-environment calls | Native via shared execution layer |
| Token model             | Unified                           |
| AOT support             | Yes (wasmtime)                    |

---

## Repository layout

### `bins/`

Binary applications:

- `fluent` — Fluent node CLI and utility entrypoint.
- `runtime-upgrade` — CLI for runtime upgrades.

### `crates/`

Core libraries:

- `build` — deterministic build tooling and artifact generation.
- `codec` / `codec-derive` — ABI-like codec and derive macros.
- `contracts` — embedded build outputs for system contracts.
- `crypto` — cryptographic primitives and runtime adapters.
- `evm` — interruptible EVM interpreter integration.
- `genesis` — genesis helpers and contract bundle metadata.
- `revm` — Fluentbase REVM integration layer.
- `runtime` — rWasm execution runtime and syscall dispatch.
- `sdk` / `sdk-derive` — developer SDK and proc-macros.
- `testing` — testing harnesses for runtime and EVM flows.
- `types` — shared types, constants, and syscall indices.

> Note: some SVM-related crates are currently excluded from the top-level workspace build and may evolve independently.

### `contracts/`

System contracts that form the genesis file for the initial blockchain setup and runtime upgrades.
These include runtimes for EVM, SVM, WASM, UST, and others.

### `examples/`

Example contracts and simple demo apps.

### `e2e/`

End-to-end tests and benchmarks.

### `flips/`

A set of FLIPs (Fluent Improvement Proposals).

---

## Versioning

Fluentbase uses the following versioning system: `<stage>.<major>.<minor>`, where:

* **stage** — indicates a major Fluentbase release (currently v1). It changes only when Fluentbase moves to a new
  development
  stage.
* **major** — used for genesis-breaking or feature-breaking updates that require a runtime upgrade to the genesis file.
  These changes must be made through a release branch and cannot be merged directly into `devel`.
* **minor** — for minor fixes that do not affect the genesis file (e.g., SDK fixes, documentation, etc.).
  Note: Some SDK fixes may cause a new genesis generation, but in such cases we do not increment the major version.

## Building & Testing

The root `Makefile` builds major modules, contracts, and examples:

```bash
make build # build contracts & genesis files
make clippy # run clippy checks
make test # run unit & e2e testing suites
make pr # run pre-pr checks (clippy+test)
```

## Running the Node

The following chain IDs are available:

* `dev` — Local development chain (`1337`)
* `fluent-devnet` — Fluent Devnet (`20993`)
* `fluent-testnet` — Fluent Testnet (`20994`)
* `fluent-mainnet` — Fluent Mainnet (`25363`)

For **Fluent Testnet**, the node must be initialized using a snapshot first:

```bash
./fluent init --datadir=./datadir --chain=fluent-testnet
./fluent download --datadir=./datadir --chain=fluent-testnet
```

To start the node, run:

```bash
./fluent --datadir=./datadir --chain=fluent-testnet
```

## Docs & Examples

You can find more documentation and examples in the official Fluent docs:
https://docs.fluent.xyz/

---

## Contributing

We welcome community contributions.
See [CONTRIBUTING.md](CONTRIBUTING.md) for details.
