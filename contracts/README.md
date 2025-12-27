# Fluentbase Contracts Workspace

This directory is a Cargo workspace that hosts a collection of smart contracts and precompiles built with the Fluentbase
SDK. It includes cryptographic precompiles commonly used in EVM environments, example contracts, and integration
utilities.

## Structure

Each subfolder is an individual crate. Notable crates include:

- blake2f — BLAKE2f hashing precompile implementation.
- bls12381 — BLS12-381 pairing-friendly curve precompile utilities.
- bn256 — BN256 (alt_bn128) precompile utilities.
- ecrecover — ECDSA public key recovery (secp256k1) precompile.
- eip2935 — EIP-2935 related utilities (historical block hashes access helpers).
- erc20 — Experimental ERC-20 token built on fluentbase-erc20.
- evm — EVM runtime for running EVM-compatible applications.
- identity — Simple identity contract.
- kzg — KZG point evaluation precompile (EIP-4844).
- modexp — Modular exponentiation precompile (EIP-198).
- multicall — Batched/multicall helper contract.
- nitro — A AWS Nitro attestation verifier.
- oauth2 — OAuth 2.0 style authentication helper contracts.
- ripemd160 — RIPEMD-160 precompile.
- secp256r1 — secp256r1 (P-256) signature verification precompile (EIP-7212).
- sha256 — SHA-256 hashing precompile.
- svm — Solana VM (SVM) integration contracts.
- wasm — A compiler form Wasm into rWasm (devnet & testnet only).
- webauthn — WebAuthn verification helpers and tests.

## Building

You can build all contracts in this workspace from the contracts directory or the repository root:

- Build all crates in this workspace:
  cargo build -p name-of-crate
  or build all:
  cargo build --workspace

- Release build tuned for WASM size/perf (see profiles in Cargo.toml):
  cargo build --workspace --release

## Testing

Many crates provide unit tests. Run tests for a specific crate:
cargo test -p name-of-crate
Or run all tests in the workspace:
cargo test --workspace

## Workspace Configuration

Shared configuration and dependencies are defined in contracts/Cargo.toml:

- readme: README.md (this file)
- Common dependencies:
    - fluentbase-sdk, fluentbase-erc20, fluentbase-evm, fluentbase-svm
    - revm-precompile (patched v82 branch)
    - solana-program-error and solana-bincode (SVM integration)
    - alloy-sol-types, serde, tiny-keccak, hex-literal, hex
- Profiles:
    - release: lto=fat, opt-level=3, panic=abort
    - dev: optimized for small WASM binaries (opt-level="z", strip symbols)

## Notes

- These crates are designed to compile to no_std/WASM-friendly targets where applicable.
- Some crates provide precompile logic compatible with EVM semantics via fluentbase-evm and revm-precompile.

## Repository

For more information, visit the repository:
https://github.com/fluentlabs-xyz/fluentbase
