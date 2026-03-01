# Fluentbase Framework

[![codecov](https://codecov.io/github/fluentlabs-xyz/fluentbase/graph/badge.svg?token=FCA43Y60LW)](https://codecov.io/github/fluentlabs-xyz/fluentbase)
[![Test](https://github.com/fluentlabs-xyz/fluentbase/actions/workflows/ci.yml/badge.svg)](https://github.com/fluentlabs-xyz/fluentbase/actions/workflows/ci.yml)

**Fluentbase** is a modular framework for building execution environments and smart contracts that compile into **rWasm** and run in a unified, proof-friendly runtime.

> Fluentbase is experimental and under active development.
> APIs, naming, and behavior can change between releases.

---

## Fluentbase + rWasm (TL;DR)

Fluentbase uses **Blended Execution**: EVM/SVM/WASM are treated as compatibility layers, while execution converges to a single intermediate representation (**rWasm**) and one provable state transition function (STF).

### Architecture

Traditional multi-VM architecture:

```text
EVM runtime | SVM runtime | WASM runtime
          \   |   /
           distinct execution semantics
                 + distinct proving surfaces
```

Fluentbase architecture:

```text
EVM / SVM / WASM adapters
           ↓
      System contracts
           ↓
         rWasm IR
           ↓
    Unified execution VM
           ↓
        ZK proof system
```

### Quick comparison

| Property | Fluentbase |
| --- | --- |
| Execution model | Unified IR |
| VM count | 1 (rWasm) |
| Environment support | EVM / SVM / WASM via adapters |
| Proof target | Single STF |
| Determinism | Strong |
| ZK efficiency | First-class constraint |
| Cross-environment calls | Native via shared execution layer |
| Token model | Unified (UST) |

### Mental model

Fluentbase is closer to **“LLVM for smart contracts + a provable runtime”** than to a traditional blockchain VM.

---

## Repository layout

### `bins/`

Binary applications:

- `fluent` — Fluent CLI and utility entrypoint.
- `chain-transition-verifier` — verifier tooling for transition validation workflows.

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
- `universal-token` — Universal Token Standard primitives.

> Note: some SVM-related crates are currently excluded from the top-level workspace build and may evolve independently.

### `contracts/`

System contracts and builtins (EVM/SVM/WASM adapters, hashing/crypto utilities, token/system modules).

### `examples/`

Example contracts and demo apps.

### `e2e/`

End-to-end tests and benchmarks.

---

## Building

The root `Makefile` builds major modules, contracts, and examples:

```bash
make
```

---

## Contributing

We welcome community contributions.
See [CONTRIBUTING.md](CONTRIBUTING.md) for details.
