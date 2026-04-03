# Fluent RPC vs Upstream Reth RPC (EVM code semantics)

This doc explains the RPC behavior differences between:

- **Upstream Reth** (`paradigmxyz/reth`, e.g. `release/v1.11.1`), and
- **Fluent Reth fork** (`fluentlabs-xyz/reth`, `v1.11-patched`)

for account code retrieval in Fluent's runtime model.

---

## TL;DR

On Fluent, some accounts store an **OwnableAccount wrapper** in state (runtime-managed metadata + payload), not plain EVM bytecode.

If RPC returns raw stored bytes for those accounts, EVM clients can read wrapper bytes instead of runtime bytecode.

So Fluent changed behavior to keep EVM-facing APIs compatible:

- `eth_getCode` → returns **runtime EVM bytecode** for Fluent runtime-owned Ownable accounts.
- `eth_getRawCode` (Fluent-added) → returns **raw stored bytes** (wrapper-preserving) for tooling that needs exact storage bytes.

---

## What changed vs upstream

## 1) `eth_getCode` semantics differ

### Upstream Reth behavior
`eth_getCode` returns `account_code(...).original_bytes()`.

### Fluent fork behavior
`eth_getCode` checks whether the bytecode is:

- `Bytecode::OwnableAccount(acc)`, and
- `acc.owner_address == fluentbase_types::PRECOMPILE_EVM_RUNTIME`

If yes, Fluent decodes `EthereumMetadata` and returns `code_copy()` (the EVM runtime bytecode).
Otherwise it falls back to original bytes.

**Implementation location (Fluent fork):**
- `reth/crates/rpc/rpc-eth-api/src/helpers/state.rs` (`LoadState::get_code`)

This is the compatibility-critical change.

---

## 2) New method: `eth_getRawCode`

Fluent added `eth_getRawCode` to preserve access to unmodified stored bytes.

Added in:
- `reth/crates/rpc/rpc-eth-api/src/core.rs`
- `reth/crates/rpc/rpc-api/src/engine.rs`
- `reth/crates/rpc/rpc/src/engine.rs`

Behavior:
- returns raw `account_code(...).original_bytes()`
- no Ownable-account unwrapping

---

## 3) `eth_getAccountInfo`

`eth_getAccountInfo` exists in both upstream and Fluent fork at this base line.
It returns `balance`, `nonce`, and `code` where `code` is read from `original_bytes()`.

So for Fluent Ownable runtime accounts, `getAccountInfo.code` can still be wrapper/raw bytes.
That is intentional for low-level state consumers and matches the method's current implementation.

---

## 4) `eth_getRawAccountInfo`

At time of writing, there is **no** `eth_getRawAccountInfo` method in `v1.11-patched`.

If you need a split:
- normalized EVM code: use `eth_getCode`
- raw stored code: use `eth_getRawCode`
- balance/nonce (+raw code bundle): use `eth_getAccountInfo`

---

## Why this exists

Fluent runtime uses Ownable account wrappers for routing/runtime ownership.
For EVM API compatibility, tooling expects `eth_getCode` to return executable/runtime bytecode, not wrapper envelopes.

Without Fluent's `eth_getCode` adjustment, clients can mis-handle contract code (e.g. decode failures, wrong bytecode fingerprinting, incorrect downstream assumptions).

`eth_getRawCode` is the escape hatch for infra/debug/indexers that need exact stored bytes.

---

## Recommended client usage

- For standard Ethereum compatibility: **prefer `eth_getCode`**.
- For forensic/indexing/storage-level inspection: **use `eth_getRawCode`**.
- Do not assume `eth_getAccountInfo.code` is equivalent to Fluent-normalized `eth_getCode` for Ownable runtime accounts.

---

## Quick check example

```bash
# EVM-compatible view
curl -s -X POST "$RPC" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_getCode","params":["0x...","latest"]}'

# Raw storage view
curl -s -X POST "$RPC" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"eth_getRawCode","params":["0x...","latest"]}'
```

If the account is Fluent runtime-owned Ownable, these can differ by design.
