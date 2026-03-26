# Universal Token Runtime (UST20)

## Runtime address and routing

- delegated runtime address: `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME = 0x0000000000000000000000000000000000520008`
- create-time routing is selected by init-code prefix `UNIVERSAL_TOKEN_MAGIC_BYTES` (`"ERC "`, `0x45524320`)
- resolver is `resolve_precompiled_runtime_from_input(...)` in `crates/types/src/genesis.rs`

When prefix matches, created account is wrapped as `OwnableAccount(owner = PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME)` in REVM create hook (`crates/revm/src/evm.rs`).

---

## Constructor payload

Constructor expects:

`UNIVERSAL_TOKEN_MAGIC_BYTES (4 bytes) + abi.encode(InitialSettings)`

`InitialSettings` (`crates/sdk/src/universal_token/storage.rs`):

1. `token_name: TokenNameOrSymbol` (`bytes32`)
2. `token_symbol: TokenNameOrSymbol` (`bytes32`)
3. `decimals: u8`
4. `initial_supply: uint256`
5. `minter: address`
6. `pauser: address`

### Name/symbol decoding behavior

`TokenNameOrSymbol::as_str()`:
- takes bytes from start of `bytes32`
- stops at first `0x00` (or full 32 bytes if none)
- validates UTF-8
- constructor returns `MalformedBuiltinParams` on invalid UTF-8

Storage writes use `write_storage_short_string` (`crates/sdk/src/types/storage.rs`) with 32-byte big-endian slot representation.

---

## Deploy behavior (`contracts/universal-token/lib.rs`)

`deploy_entry`:

- requires input size `>= 4`
- decodes constructor payload via `InitialSettings::decode_with_prefix`
- writes:
  - name, symbol, decimals
  - optional `MINTER_STORAGE_SLOT` (if `minter != 0`)
  - optional `PAUSER_STORAGE_SLOT` (if `pauser != 0`)
- if `initial_supply > 0`:
  - credits deployer balance
  - sets total supply
  - emits `Transfer(0x0, deployer, initial_supply)`

On non-zero EVM exit code from constructor logic, runtime writes ABI error payload and exits with `ExitCode::Panic`.

---

## Runtime methods (selector dispatch)

Implemented in `main_entry` by 4-byte selector switch:

### Core
- `name()`
- `symbol()`
- `decimals()`
- `totalSupply()`
- `balance()` (caller balance convenience)
- `balanceOf(address)`
- `transfer(address,uint256)`
- `transferFrom(address,address,uint256)`
- `approve(address,uint256)`
- `allowance(address,address)`

### Optional privileged extensions
- `mint(address,uint256)`
- `burn(address,uint256)`
- `pause()`
- `unpause()`

### Reserved
- `token2022()` selector constant exists but is not dispatched in current runtime.

Unknown selector path returns `ERR_UST_UNKNOWN_METHOD`.

---

## Access control and freeze semantics

### Mint/burn
- if configured minter is zero => `ERR_UST_NOT_MINTABLE`
- caller must equal configured minter => else `ERR_UST_MINTER_MISMATCH`
- mint rejects zero receiver (`ERR_ERC20_INVALID_RECEIVER`)
- burn rejects zero `from` (`ERR_ERC20_INVALID_SENDER`)

### Pause/unpause
- if configured pauser is zero => `ERR_UST_NOT_PAUSABLE`
- caller must equal pauser => else `ERR_UST_PAUSER_MISMATCH`
- `pause()` while already paused => `ERR_PAUSABLE_ENFORCED_PAUSE`
- `unpause()` while not paused => `ERR_PAUSABLE_EXPECTED_PAUSE`

### Frozen-state restrictions
When frozen (`CONTRACT_FROZEN_STORAGE_SLOT != 0`), these return `ERR_PAUSABLE_ENFORCED_PAUSE`:
- `transfer`
- `transferFrom`
- `mint`
- `burn`

---

## Storage layout (slots)

Slot constants are in `crates/sdk/src/universal_token/consts.rs`:

- `TOTAL_SUPPLY_STORAGE_SLOT`
- `MINTER_STORAGE_SLOT`
- `PAUSER_STORAGE_SLOT`
- `CONTRACT_FROZEN_STORAGE_SLOT`
- `SYMBOL_STORAGE_SLOT`
- `NAME_STORAGE_SLOT`
- `DECIMALS_STORAGE_SLOT`
- `ALLOWANCE_STORAGE_SLOT` (nested mapping)
- `BALANCE_STORAGE_SLOT` (mapping)

Mapping keys use SDK map-slot derivation (`MapKey::compute_slot`).

---

## Notes about compatibility paths

`LegacyInitialSettings` decoding remains in `erc20_compute_deploy_storage_keys(...)` for storage prefetch compatibility, but constructor path uses `InitialSettings`.

`PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME` is included in:
- delegated runtime set,
- system-runtime execution set,
- engine-metered precompile set.
