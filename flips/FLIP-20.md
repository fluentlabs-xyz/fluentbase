# FLIP-20: Universal Token Standard (ERC-20 + SVM/SPL Interop)

- **FLIP**: 20
- **Title**: Universal Token Standard
- **Author**: Fluentbase Core Contributors
- **Status**: Draft
- **Type**: Standards Track
- **Category**: Token
- **Created**: 2026-03-15
- **Requires**: ERC-20 ABI compatibility, Fluent precompiled runtime routing

---

## Abstract

FLIP-20 defines a canonical fungible-token standard for Fluentbase with:

1. ERC-20 ABI compatibility for EVM tooling and contracts,
2. optional role-gated mint/pause extensions,
3. precompiled runtime execution for deterministic, low-overhead performance,
4. SVM/SPL interoperability so the same asset model can be used across EVM and SVM-facing flows.

`UST20` is the reference FLIP-20-compatible implementation.

---

## Canonical constants

The following constants are normative for this FLIP:

- `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME = 0x0000000000000000000000000000000000520008`
- `UNIVERSAL_TOKEN_MAGIC_BYTES = 0x45524320` (`"ERC "`)

Selector/hash convention used in this document:

- Function selectors and custom error codes are the first 4 bytes of `keccak256(<signature>)`.

---

## Motivation

In EVM context, ERC-20 tokens are usually separate bytecode deployments even when they implement the same behavior. That fragmentation prevents deep runtime-level optimization and duplicates execution overhead.

By using one shared runtime for ERC20/SPL token flows, Fluentbase can apply AOT-style optimization (e.g., Wasmtime-like strategies) and amortize it across all FLIP-20 tokens. UST20 solves this by preserving ERC-20 compatibility while standardizing execution on a single high-performance runtime path.

FLIP-20 therefore targets a token standard that is:

- **developer-familiar** (ERC-20 surface),
- **cross-environment** (EVM + SVM/SPL interoperability),
- **fast by default** (shared precompiled runtime path),
- **operationally consistent** (uniform errors, events, and storage semantics).

---

## Specification

### 1) Runtime model

A FLIP-20 token MUST execute via a delegated runtime/precompile model:

- Runtime address: `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME = 0x0000000000000000000000000000000000520008`
- Deployed token instances MUST have isolated storage per token address.
- Runtime logic MAY be shared across all FLIP-20 token instances.

This model enables host/runtime-level optimization, including hardware-accelerated paths where available.

### 2) Deployment format

A compliant deployment payload MUST be:

```text
UNIVERSAL_TOKEN_MAGIC_BYTES (4 bytes)
+ ABI.encode(InitialSettings)
```

`InitialSettings` fields (in order):

1. `token_name` (fixed 32-byte representation)
2. `token_symbol` (fixed 32-byte representation)
3. `decimals` (`uint8` ABI word)
4. `initial_supply` (`uint256`)
5. `minter` (`address`)
6. `pauser` (`address`)

`token_name` and `token_symbol` encoding rule (normative):

- Values MUST be encoded as `bytes32` with ASCII/UTF-8 bytes placed in the most-significant portion of the 32-byte word (left-aligned / big-endian orientation).
- Remaining trailing bytes MUST be zero (`0x00`) padded.
- Right-aligned string packing is non-compliant.

If `initial_supply > 0`, constructor/deploy logic MUST mint the initial amount to the deployer (`contract_caller`) and emit `Transfer(address(0), deployer, initial_supply)`.

### 3) Required interface

A FLIP-20 token MUST expose the following methods with ERC-20-compatible semantics:

- `name() -> string`
- `symbol() -> string`
- `decimals() -> uint8`
- `totalSupply() -> uint256`
- `balanceOf(address owner) -> uint256`
- `transfer(address to, uint256 amount) -> bool`
- `transferFrom(address from, address to, uint256 amount) -> bool`
- `approve(address spender, uint256 amount) -> bool`
- `allowance(address owner, address spender) -> uint256`

Known canonical selectors used by FLIP-20 reference implementation:

- `name()` = `0x06fdde03`
- `symbol()` = `0x95d89b41`
- `decimals()` = `0x313ce567`
- `totalSupply()` = `0x18160ddd`
- `balanceOf(address)` = `0x70a08231`
- `transfer(address,uint256)` = `0xa9059cbb`
- `transferFrom(address,address,uint256)` = `0x23b872dd`
- `approve(address,uint256)` = `0x095ea7b3`
- `allowance(address,address)` = `0xdd62ed3e`

### 4) Optional extensions

A FLIP-20 implementation MAY expose:

- `balance() -> uint256` (caller convenience)
- `mint(address to, uint256 amount) -> bool`
- `burn(address from, uint256 amount) -> bool`
- `pause() -> bool`
- `unpause() -> bool`

Extension behavior requirements:

- If `minter == address(0)`, mint/burn functionality MUST be disabled.
- If `pauser == address(0)`, pause/unpause functionality MUST be disabled.
- Mutating methods MUST reject static context execution.
- On success, mutating methods MUST return ABI-encoded `true` (32-byte word `1`).

### 5) Event requirements

Compliant implementations MUST emit ERC-20-compatible events:

- `Transfer(address indexed from, address indexed to, uint256 amount)`
- `Approval(address indexed owner, address indexed spender, uint256 amount)`

If pause extensions are implemented, they SHOULD emit:

- `Paused(address pauser)`
- `Unpaused(address pauser)`

### 6) Error and revert semantics

Implementations SHOULD provide deterministic, typed errors. The reference FLIP-20/UST20 error set and selectors are:

| Constant | Error signature | Selector |
|---|---|---|
| `ERR_UST_UNKNOWN_METHOD` | `USTUnknownMethod(bytes4)` | `0xb0d8e5d7` |
| `ERR_UST_NOT_PAUSABLE` | `USTNotPausable()` | `0x0507e61c` |
| `ERR_UST_PAUSER_MISMATCH` | `USTPauserMismatch(address)` | `0xbb8db808` |
| `ERR_UST_NOT_MINTABLE` | `USTNotMintable()` | `0x9f1090b2` |
| `ERR_UST_MINTER_MISMATCH` | `USTMinterMismatch(address)` | `0xf5143e51` |
| `ERR_ERC20_INSUFFICIENT_BALANCE` | `ERC20InsufficientBalance(address,uint256,uint256)` | `0xe450d38c` |
| `ERR_ERC20_INVALID_SENDER` | `ERC20InvalidSender(address)` | `0x96c6fd1e` |
| `ERR_ERC20_INVALID_RECEIVER` | `ERC20InvalidReceiver(address)` | `0xec442f05` |
| `ERR_ERC20_INSUFFICIENT_ALLOWANCE` | `ERC20InsufficientAllowance(address,uint256,uint256)` | `0xfb8f41b2` |
| `ERR_ERC20_INVALID_APPROVER` | `ERC20InvalidApprover(address)` | `0xe602df05` |
| `ERR_ERC20_INVALID_SPENDER` | `ERC20InvalidSpender(address)` | `0x94280d62` |
| `ERR_PAUSABLE_ENFORCED_PAUSE` | `EnforcedPause()` | `0xd93c0665` |
| `ERR_PAUSABLE_EXPECTED_PAUSE` | `ExpectedPause()` | `0x8dfc202b` |

State mutations and logs MUST be reverted on failed execution.

### 7) SVM/SPL interoperability

FLIP-20 defines cross-environment interoperability as a first-class requirement:

- An implementation MUST preserve ERC-20 ABI behavior for EVM callers.
- Implementations SHOULD support SVM/SPL interoperability adapters (e.g. Token-2022 routing paths) so SPL-side flows can consume the same underlying token state semantics.
- Interoperability MUST preserve fungibility and accounting invariants (`totalSupply`, balances, allowances/authorities).

Result: FLIP-20 assets are inter-tradable across EVM and SPL-facing user flows, while keeping an ERC-20 interface for EVM tooling.

---

## Performance profile (reference benchmark)

Latest benchmark snapshot (1000 measurements, transfer path):

| Runtime path | Transfer time (median) | Deployment gas |
|---|---:|---:|
| Original EVM ERC20 | ~4.6528 µs | 878,183 |
| Emulated EVM ERC20 | ~20.932 µs | 878,183 |
| rWasm Contract ERC20 | ~126.39 µs | 3,769,621 |
| Precompiled Universal Token (FLIP-20/UST20) | ~6.2704 µs | 74,122 |

Key takeaways:

- UST20/FLIP-20 precompiled deployment gas is dramatically lower than bytecode deployments.
- Transfer latency is near native EVM baseline and significantly faster than emulated/rWasm contract paths.
- The precompiled model is designed for super-fast execution and can benefit from host-level/hardware acceleration.

---

## Reference implementation status

`contracts/universal-token` in Fluentbase is the reference FLIP-20 implementation (`UST20`):

- selector-based dispatch,
- ERC-20 core surface,
- optional mint/pause extensions,
- role-gated controls,
- deterministic storage layout,
- precompiled runtime routing.

UST20 is FLIP-20 compatible.

---

## Security considerations

Implementers MUST:

- enforce access control for role-gated operations,
- preserve atomicity (no partial writes on failure),
- reject state changes during static execution,
- preserve event/state consistency under revert,
- guard arithmetic overflow/underflow,
- treat cross-environment adapters as consensus-critical code paths.

---

## Backward compatibility

FLIP-20 is backward-compatible with ERC-20 integrations at ABI level for required methods/events.
Optional extensions are additive.

---

## Rationale

FLIP-20 standardizes token behavior once, then reuses it everywhere:

- one runtime model,
- one compliance target,
- one interop story for EVM and SVM/SPL,
- one performance-oriented execution path.

This reduces fragmentation and gives ecosystem tooling a stable, serious standard to build against.
