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

1. `token_name` (`bytes32`, UTF-8 canonical codec; see below)
2. `token_symbol` (`bytes32`, UTF-8 canonical codec; see below)
3. `decimals` (`uint8` ABI word)
4. `initial_supply` (`uint256`)
5. `minter` (`address`)
6. `pauser` (`address`)

Canonical `bytes32` text codec for `token_name` and `token_symbol` (normative):

- Fields MUST be treated as UTF-8 encoded byte sequences stored in fixed 32-byte words.
- Encoding MUST place bytes in the most-significant portion of the 32-byte word (left-aligned / big-endian orientation) and pad the remaining bytes with trailing `0x00`.
- Right-aligned string packing is non-compliant.
- Decoding to `name()` / `symbol()` MUST locate the first `0x00` byte and decode only the prefix before it as UTF-8; bytes from the first `0x00` onward are ignored.
- For canonical payloads (no interior nulls), this is equivalent to stripping trailing `0x00` bytes; payloads with interior `0x00` are non-canonical and are truncated at the first `0x00`.
- Non-UTF-8 decoded byte sequences MUST be rejected (no replacement policy).
- Overflow behavior: if input text exceeds 32 bytes, encoding truncates on the right to fit 32 bytes (byte-wise); if truncation yields invalid UTF-8 at decode time, initialization MUST fail.

If `initial_supply > 0`, constructor/deploy logic MUST mint the initial amount to the deployer (`contract_caller`) and emit `Transfer(address(0), deployer, initial_supply)`.

### 3) Required interface

A FLIP-20 token MUST expose the following methods with ERC-20-compatible semantics:

- `name() -> string` (decoded from `token_name` via the canonical `bytes32` UTF-8 codec above)
- `symbol() -> string` (decoded from `token_symbol` via the canonical `bytes32` UTF-8 codec above)
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

Implementations SHOULD provide deterministic, typed errors. The reference FLIP-20/UST20 error set, selectors, and usage rules are:

| Constant | Error signature | Selector | When to use |
|---|---|---|---|
| `ERR_UST_UNKNOWN_METHOD` | `USTUnknownMethod(bytes4)` | `0xb0d8e5d7` | MUST be returned when the 4-byte selector does not match any supported FLIP-20 method. |
| `ERR_UST_NOT_PAUSABLE` | `USTNotPausable()` | `0x0507e61c` | MUST be returned when `pause()` or `unpause()` is called but pausing is disabled (no pauser configured). |
| `ERR_UST_PAUSER_MISMATCH` | `USTPauserMismatch(address)` | `0xbb8db808` | MUST be returned when `pause()` / `unpause()` is called by an address different from the configured pauser. |
| `ERR_UST_NOT_MINTABLE` | `USTNotMintable()` | `0x9f1090b2` | MUST be returned when `mint()` or privileged `burn()` is called but minting is disabled (no minter configured). |
| `ERR_UST_MINTER_MISMATCH` | `USTMinterMismatch(address)` | `0xf5143e51` | MUST be returned when `mint()` or privileged `burn()` is called by an address different from the configured minter. |
| `ERR_ERC20_INSUFFICIENT_BALANCE` | `ERC20InsufficientBalance(address,uint256,uint256)` | `0xe450d38c` | MUST be returned when the source balance (or burn source/total-supply subtraction) is smaller than the requested amount. |
| `ERR_ERC20_INVALID_SENDER` | `ERC20InvalidSender(address)` | `0x96c6fd1e` | MUST be returned when the effective sender/source address is invalid for the operation (e.g., zero address). |
| `ERR_ERC20_INVALID_RECEIVER` | `ERC20InvalidReceiver(address)` | `0xec442f05` | MUST be returned when a transfer/mint target address is invalid (e.g., zero address). |
| `ERR_ERC20_INSUFFICIENT_ALLOWANCE` | `ERC20InsufficientAllowance(address,uint256,uint256)` | `0xfb8f41b2` | MUST be returned when `transferFrom` allowance is less than the requested spend amount (except unlimited allowance semantics). |
| `ERR_ERC20_INVALID_APPROVER` | `ERC20InvalidApprover(address)` | `0xe602df05` | SHOULD be returned when an approve-style operation receives an invalid owner/approver address under the implementation policy. |
| `ERR_ERC20_INVALID_SPENDER` | `ERC20InvalidSpender(address)` | `0x94280d62` | SHOULD be returned when an approve-style operation receives an invalid spender/delegate address under the implementation policy. |
| `ERR_PAUSABLE_ENFORCED_PAUSE` | `EnforcedPause()` | `0xd93c0665` | MUST be returned when a state-mutating operation is attempted while the token is paused (including duplicate `pause()`). |
| `ERR_PAUSABLE_EXPECTED_PAUSE` | `ExpectedPause()` | `0x8dfc202b` | MUST be returned when `unpause()` is called while the token is not paused. |

State mutations and logs MUST be reverted on failed execution.

### 7) SVM/SPL interoperability

FLIP-20 defines cross-environment interoperability as a first-class requirement:

- An implementation MUST preserve ERC-20 ABI behavior for EVM callers.
- Implementations MUST support SVM/SPL interoperability adapters (e.g. Token-2022 routing paths) so SPL-side flows can consume the same underlying token state semantics.
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

### Benchmark provenance

For the **Latest benchmark snapshot** table above (Original EVM ERC20, Emulated EVM ERC20, rWasm Contract ERC20, Precompiled Universal Token):

- Snapshot source: user-supplied benchmark image (received 2026-03-15 UTC).
- Snapshot timestamp in artifact: not explicitly recorded in the image.
- Code under test (repro baseline): `origin/devel` at `f9e008b6`.
- Runtime components:
  - EVM path: `fluentbase-revm` (workspace dependency)
  - rWasm path: `rwasm` from `fluentlabs-xyz/rwasm` branch `devel`
- Toolchain target (from repo): `rust-toolchain = 1.92.0`.
- Measurement tool: Criterion `0.8.1` (`e2e/Cargo.toml`).
- Benchmark harness knobs (`e2e/benches/erc20.rs`): warmup `500ms`, measurement `1s`, sample size `1000`.
- Reproduction command:
  - `cargo bench -p fluentbase-e2e --bench erc20`
- Raw result artifact path (default Criterion output):
  - `target/criterion/Tokens Transfer Comparison/`
- Repro environment profile captured while preparing this FLIP update:
  - OS/kernel: `Linux 6.8.0-90-generic`
  - CPU: `AMD EPYC-Milan Processor` (4 vCPU)
  - RAM: `15 GiB`

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
