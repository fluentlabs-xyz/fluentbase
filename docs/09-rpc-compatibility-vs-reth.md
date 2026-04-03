# Fluent RPC vs Upstream Reth RPC (code/account behavior)

This note explains how Fluent’s RPC behavior differs from upstream Reth for account and code reads.

## Context

Fluent stores some contracts in a runtime-managed wrapped format (Ownable account layout). That layout contains metadata plus payload.

Most Ethereum clients expect RPC results that look like plain EVM account/code semantics.

Because of that, Fluent exposes **two views** in RPC:

- a compatibility view (normalized for EVM clients), and
- a raw view (exact storage-level representation).

## Method matrix (Fluent fork)

### Code endpoints

- **`eth_getCode`**: compatibility/normalized view.
  - For Fluent runtime-owned wrapped accounts, returns extracted EVM runtime bytecode.
  - For normal accounts, behaves like upstream.

- **`eth_getRawCode`**: raw view.
  - Returns stored bytes as-is.
  - No runtime unwrapping.

### Account endpoints

- **`eth_getAccount`**: compatibility/normalized view.
  - For runtime-owned wrapped accounts, adjusts returned code hash to match the extracted EVM bytecode hash.
  - Keeps account-shaped output expected by Ethereum tooling.

- **`eth_getRawAccount`**: raw view.
  - Returns account fields based on storage-level representation.
  - No normalization for wrapped runtime accounts.

### AccountInfo endpoints

- **`eth_getAccountInfo`**: compatibility-oriented view. For runtime-owned wrapped accounts, the returned code payload is intended to reflect EVM-facing semantics.
- **`eth_getRawAccountInfo`**: raw/storage-oriented view, intended for users who need account data without EVM compatibility mapping.

Why keep both:

- compatibility consumers (wallets, app SDKs, generic Ethereum tooling) generally want normalized EVM semantics,
- infra/verification consumers often need storage-truth semantics.

Typical raw-account-info use cases:

- building or validating account/state proofs against storage-level representation,
- cross-checking state root / witness pipelines,
- indexer pipelines that must preserve canonical stored bytes,
- debugging mismatches between runtime-mapped bytecode and persisted account payload,
- fork/fork-db tooling where cache keys or bytecode identity must match storage bytes exactly.

> **Current status (`v1.11-patched`):** helper implementations are already split (`get_account_info` vs `get_raw_account_info`), while the top-level RPC handler is still aligned to the normalized path. This is being updated so `eth_getRawAccountInfo` maps to the raw helper path as intended.

## Difference vs upstream Reth

Upstream baseline does not carry Fluent’s wrapped-account normalization behavior. Fluent extends behavior so Ethereum-facing clients receive expected EVM-compatible values on default methods, while still providing raw methods for infra/debug needs.

## Why Fluent implemented this

Without this split, Ethereum clients can misread wrapper bytes as contract bytecode and produce incorrect downstream behavior (tooling decode assumptions, bytecode matching, etc.).

Fluent’s model is:

- **default method names** (`getCode`, `getAccount`, `getAccountInfo`) lean compatibility-first,
- **raw method names** (`getRawCode`, `getRawAccount`, `getRawAccountInfo`) expose storage-level truth.

## Recommended usage

- Use **`eth_getCode` / `eth_getAccount` / `eth_getAccountInfo`** for normal app/tool compatibility.
- Use **`eth_getRawCode` / `eth_getRawAccount` / `eth_getRawAccountInfo`** when you explicitly need storage-level bytes and hashes.
