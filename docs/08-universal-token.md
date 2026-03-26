# Universal Token Runtime (UST20)

## What this runtime is

Universal Token is a delegated system runtime for fungible tokens.

The key design choice is:
- many token instances,
- one shared runtime implementation,
- isolated per-token state.

That gives predictable behavior and operationally simpler upgrades compared to many independent token implementations.

---

## Address and routing

Runtime address:

- `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME = 0x0000000000000000000000000000000000520008`

Create-time routing is selected by init payload prefix:

- `UNIVERSAL_TOKEN_MAGIC_BYTES = "ERC " (0x45524320)`

If prefix matches, new account is wrapped and delegated to universal-token runtime.

---

## Constructor input format

Constructor expects:

`magic(4 bytes) + abi.encode(InitialSettings)`

`InitialSettings` fields:

1. token name (`bytes32`-style fixed text)
2. token symbol (`bytes32`-style fixed text)
3. decimals
4. initial supply
5. minter address (optional role)
6. pauser address (optional role)

### Text behavior (name/symbol)

- bytes are read from start of 32-byte field,
- decode stops at first `0x00`,
- invalid UTF-8 is rejected,
- constructor fails on malformed text payload.

---

## Deploy behavior

On successful deploy:

- stores name/symbol/decimals,
- stores minter/pauser only when non-zero,
- if initial supply > 0:
  - mints to deployer,
  - updates total supply,
  - emits ERC20 `Transfer(0x0 -> deployer)`.

If constructor logic returns non-zero EVM-style error code, runtime writes ABI error payload and exits with panic-class host exit.

---

## Supported method surface

### Core ERC20-like methods
- `name`, `symbol`, `decimals`
- `totalSupply`
- `balance`, `balanceOf`
- `transfer`, `transferFrom`
- `approve`, `allowance`

### Optional privileged methods
- `mint`
- `burn`
- `pause`
- `unpause`

Unknown selector returns `ERR_UST_UNKNOWN_METHOD`.

---

## Role and freeze semantics

### Mint/Burn
- no minter configured -> `ERR_UST_NOT_MINTABLE`
- caller not minter -> `ERR_UST_MINTER_MISMATCH`
- mint to zero -> `ERR_ERC20_INVALID_RECEIVER`
- burn from zero -> `ERR_ERC20_INVALID_SENDER`

### Pause/Unpause
- no pauser configured -> `ERR_UST_NOT_PAUSABLE`
- caller not pauser -> `ERR_UST_PAUSER_MISMATCH`
- pause while already paused -> `ERR_PAUSABLE_ENFORCED_PAUSE`
- unpause while not paused -> `ERR_PAUSABLE_EXPECTED_PAUSE`

### Frozen-state restrictions
When frozen, transfer/mint/burn paths are blocked with enforced-pause error.

---

## Storage model

Runtime stores:

- total supply,
- role addresses (minter/pauser),
- frozen flag,
- name/symbol/decimals,
- balances map,
- allowances nested map.

Slots are deterministic constants; mapping keys are derived by SDK slot derivation helpers.

---

## Compatibility notes

- Legacy constructor payload compatibility still exists in storage prefetch helpers.
- `token2022()` selector constant exists but is currently reserved (not dispatched).
- Universal token runtime is part of:
  - delegated-runtime set,
  - system-runtime execution set,
  - engine-metered precompile set.
