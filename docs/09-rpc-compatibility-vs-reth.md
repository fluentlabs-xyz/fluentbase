# Fluent RPC vs Upstream Reth RPC (code retrieval behavior)

This note explains how Fluent’s RPC behavior differs from upstream Reth when clients read account code.

## Context

In Fluent, some accounts are stored in a wrapped form used by the runtime (often called an Ownable account layout). That wrapper contains metadata plus payload.

Most Ethereum tools, however, expect `eth_getCode` to return the contract’s executable EVM bytecode directly.

If an RPC server returns wrapper bytes to those tools, integrations can break or behave incorrectly.

## What is different

### 1) `eth_getCode` in Fluent is compatibility-oriented

- **Upstream Reth (baseline):** returns the stored code bytes as-is.
- **Fluent fork:** for Fluent runtime-owned wrapped accounts, it returns the extracted EVM bytecode instead of wrapper bytes.

In plain terms: Fluent makes `eth_getCode` behave the way Ethereum tooling expects, even when internal storage is wrapped.

### 2) Fluent adds `eth_getRawCode`

Fluent added `eth_getRawCode` as a separate endpoint for infrastructure users who need the exact bytes from storage (including wrapper form).

So you now have two views:

- **Normalized EVM view:** `eth_getCode`
- **Raw storage view:** `eth_getRawCode`

### 3) `eth_getAccountInfo` remains a low-level payload view

`eth_getAccountInfo` is present in both upstream and Fluent at this base level. It returns balance, nonce, and code payload.

For wrapped runtime accounts, that code field can still reflect raw/stored representation.

So do not assume `getAccountInfo.code` is always equivalent to Fluent’s normalized `getCode` output.

### 4) About `eth_getRawAccountInfo`

At the current patched baseline, there is no separate `eth_getRawAccountInfo` method.

## Why Fluent implemented this

Fluent added these semantics to preserve practical EVM compatibility for wallets, SDKs, tooling, and indexers that rely on `eth_getCode` behaving like “runtime bytecode”.

Without this split:

- clients may read wrapper bytes as if they were contract bytecode,
- bytecode matching/fingerprinting can fail,
- downstream execution assumptions can be wrong.

`eth_getRawCode` exists so low-level tooling still has a direct path to storage-accurate bytes.

## Recommended usage

- Use **`eth_getCode`** for normal Ethereum-compatible app behavior.
- Use **`eth_getRawCode`** for debugging, indexing internals, and storage-level inspection.
- Treat **`eth_getAccountInfo.code`** as low-level account payload, not guaranteed normalized runtime bytecode.
