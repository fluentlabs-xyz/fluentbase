# Universal Token (UST20) in Fluentbase

This document explains how the Universal Token contract works in Fluentbase, how deployment routing works, and how to deploy custom tokens using `CREATE` and `CREATE2`.

---

## What is UST20?

`UST20` is Fluentbase’s ERC-20 style universal token runtime.

It is implemented as a shared system runtime with per-token isolated storage, so each token gets its own state while reusing the same optimized execution path.

The delegated runtime address for universal token is:

- `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME = 0x0000000000000000000000000000000000520008`

UST tokens are **not deployed as unique bytecode implementations**. Instead, deployment input selects the shared runtime, and each deployed token keeps its own isolated storage.

## Why UST20 is useful

UST20 is designed to be both fast and composable across Fluentbase environments.

- **Fast execution path:** all UST20 tokens run through one optimized runtime instead of N custom token implementations.
- **Seamless cross-environment UX:** the same token standard can be used across EVM/SVM/WASM-facing flows in Fluentbase.
- **Unified liquidity semantics:** users can interact with assets through familiar interfaces (e.g. ERC-20/SPL-style integrations) without VM-to-VM bridge friction inside the Fluent execution model.

In practice, this gives a more native experience for cross-environment trading and app composition (e.g. ERC-20-side apps interacting with SPL-facing flows over the same underlying asset model).

---

## How deployment works

When a contract is created (`CREATE` or `CREATE2`), Fluentbase runs an internal create hook:

1. Reads `init_code` from create input.
2. Calls `resolve_precompiled_runtime_from_input(init_code)`.
3. If input starts with universal token magic bytes, runtime resolves to `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME`.
4. New account is wired as an EIP-7702 ownable account pointing to that runtime.
5. Original `init_code` is forwarded as constructor input to runtime `deploy_entry`.

For UST, the resolver checks the init code prefix against `UNIVERSAL_TOKEN_MAGIC_BYTES`.

---

## Constructor payload format (critical)

UST deployment payload must be:

```text
UNIVERSAL_TOKEN_MAGIC_BYTES (4 bytes)
+ abi.encode(InitialSettings)
```

`InitialSettings` structure:

```rust
struct InitialSettings {
    token_name: TokenNameOrSymbol,   // fixed 32 bytes, values longer than 32 bytes are truncated
    token_symbol: TokenNameOrSymbol, // fixed 32 bytes, values longer than 32 bytes are truncated
    decimals: u8,
    initial_supply: U256,
    minter: Address,
    pauser: Address,
}
```

`TokenNameOrSymbol` is a fixed 32-byte field (zero-padded UTF-8 bytes), **not dynamic Solidity string**.

Important: in the current implementation, `TokenNameOrSymbol::from_str` clamps input length to 32 bytes (`min(32, value.len())`).
So if `token_name` or `token_symbol` is longer than 32 bytes, it is **silently truncated** to the first 32 bytes.

### ABI-level field layout

Inside `abi.encode(InitialSettings)` (tuple), fields are encoded as static words:

1. `token_name` -> `bytes32` (longer inputs are truncated to 32 bytes)
2. `token_symbol` -> `bytes32` (longer inputs are truncated to 32 bytes)
3. `decimals` -> `uint8` (ABI word-padded)
4. `initial_supply` -> `uint256`
5. `minter` -> `address` (ABI word-padded)
6. `pauser` -> `address` (ABI word-padded)

Then prepend 4-byte universal token magic prefix.

---

## Runtime behavior summary

The runtime dispatches by 4-byte selectors and implements ERC-20-style methods plus optional role-based extensions.

Core selectors:

- `symbol()`
- `name()`
- `decimals()`
- `totalSupply()`
- `balance()`
- `balanceOf(address)`
- `transfer(address,uint256)`
- `transferFrom(address,address,uint256)`
- `approve(address,uint256)`
- `allowance(address,address)`
- `mint(address,uint256)` (role-gated)
- `burn(address,uint256)` (role-gated)
- `pause()` / `unpause()` (role-gated)

Role/plugin model:

- `minter == 0x0` => mint/burn disabled
- `pauser == 0x0` => pause/unpause disabled

Constructor behavior (`deploy_entry`):

- stores name/symbol/decimals
- mints `initial_supply` to deployer (`contract_caller`) if non-zero
- sets minter/pauser if provided

---

## CREATE deployment example

### Solidity-style (raw create)

```solidity
bytes memory encodedSettings = abi.encode(
    bytes32("MyToken"),
    bytes32("MTK"),
    uint8(18),
    uint256(1_000_000 ether),
    address(0x1234...), // minter or address(0)
    address(0x5678...)  // pauser or address(0)
);

bytes memory initCode = bytes.concat(
    hex"<UNIVERSAL_TOKEN_MAGIC_4B>",
    encodedSettings
);

address token;
assembly {
    token := create(0, add(initCode, 0x20), mload(initCode))
    if iszero(token) { revert(0, 0) }
}
```

### Rust SDK helper (recommended)

SDK exposes helpers:

- `create_deployment_tx(...)`
- `create_deployment_tx_with_roles(...)`
- `TokenConfig::create_deployment_transaction()`

These helpers generate the correct `magic + encoded(InitialSettings)` payload.

---

## CREATE2 deployment example

`CREATE2` works exactly the same payload-wise; only address derivation differs.

```solidity
bytes32 salt = keccak256("my-ust-v1");
bytes memory initCode = /* magic + abi.encode(InitialSettings) */;

address token;
assembly {
    token := create2(0, add(initCode, 0x20), mload(initCode), salt)
    if iszero(token) { revert(0, 0) }
}
```

Deterministic address formula:

```text
address = keccak256(0xff ++ deployer ++ salt ++ keccak256(init_code))[12:]
```

Where `init_code` here is exactly `UNIVERSAL_TOKEN_MAGIC_BYTES || abi.encode(InitialSettings)`.

---

## Method input encoding for calls

Runtime expects standard selector + ABI args:

```text
4-byte selector (big-endian)
+ ABI-encoded args
```

Examples:

- `transfer(address to, uint256 amount)`
- `approve(address spender, uint256 amount)`
- `mint(address to, uint256 amount)`

Helper command structs encode this format via `encode_for_send`.

---

## Storage shape (high-level)

Important slots:

- total supply
- minter
- pauser
- contract frozen flag
- name/symbol/decimals
- allowance mapping
- balance mapping

Slots are computed via ERC-7201-style slot derivation helpers.

---

## Common deployment mistakes

1. Missing 4-byte magic prefix.
2. Using dynamic `string` ABI for name/symbol instead of fixed 32-byte representation expected by runtime.
3. Assuming long names/symbols are preserved; in current implementation they are silently truncated to 32 bytes.
4. Wrong field order in `InitialSettings`.
5. Assuming token bytecode differs per deployment (runtime is shared; storage is per-address).
6. Forgetting that `initial_supply` mints to deployer (`contract_caller`) at deploy time.

---

## Quick checklist

- [ ] Prefix starts with `UNIVERSAL_TOKEN_MAGIC_BYTES`
- [ ] Params encoded as `InitialSettings` in exact order
- [ ] Name/symbol packed to 32-byte fields (anything >32 bytes will be silently truncated)
- [ ] Role addresses set to zero if feature should be disabled
- [ ] For deterministic deployment, use `CREATE2` with fixed salt + fixed init_code
