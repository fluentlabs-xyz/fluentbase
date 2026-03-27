# Universal Token Runtime (UST20)

## What UST20 is

UST20 is Fluentbase’s shared token runtime.

Instead of deploying a unique token implementation every time, each token instance is routed to the same runtime logic and keeps its own isolated storage.

Practical result:
- consistent token behavior,
- easier upgrades and audits,
- lower operational complexity than many independent token implementations.

---

## Routing and address

Universal token runtime address:

- `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME = 0x0000000000000000000000000000000000520008`

Create-time routing is selected by constructor prefix:

- `UNIVERSAL_TOKEN_MAGIC_BYTES = 0x45524320` (`"ERC "`)

If init code starts with this 4-byte magic, deployment is routed to the universal token runtime.

---

## Constructor payload format

Deployment payload must be:

`0x45524320 ++ abi.encode(InitialSettings)`

`InitialSettings` fields (in order):

1. `token_name` (`bytes32`)
2. `token_symbol` (`bytes32`)
3. `decimals` (`uint8` ABI word)
4. `initial_supply` (`uint256`)
5. `minter` (`address`)
6. `pauser` (`address`)

### Name/symbol text rules

Name and symbol are fixed 32-byte fields.

Current behavior:
- read from the start of `bytes32`,
- stop at first `0x00`,
- validate UTF-8,
- reject malformed UTF-8.

So constructor data should encode human text in the front of the 32-byte value, with zero-padding tail.

---

## Deploy behavior

On successful deploy, runtime:

- stores name/symbol/decimals,
- stores minter and pauser only if non-zero,
- if `initial_supply > 0`:
  - credits deployer balance,
  - sets total supply,
  - emits ERC20 `Transfer(address(0), deployer, initial_supply)`.

---

## Runtime method surface

### Core methods
- `name()`
- `symbol()`
- `decimals()`
- `totalSupply()`
- `balance()`
- `balanceOf(address)`
- `transfer(address,uint256)`
- `transferFrom(address,address,uint256)`
- `approve(address,uint256)`
- `allowance(address,address)`

### Optional privileged methods
- `mint(address,uint256)`
- `burn(address,uint256)`
- `pause()`
- `unpause()`

Unknown selector returns `ERR_UST_UNKNOWN_METHOD`.

---

## Roles and freeze behavior

### Mint/Burn
- no minter configured -> `ERR_UST_NOT_MINTABLE`
- caller != minter -> `ERR_UST_MINTER_MISMATCH`
- mint to zero -> `ERR_ERC20_INVALID_RECEIVER`
- burn from zero -> `ERR_ERC20_INVALID_SENDER`

### Pause/Unpause
- no pauser configured -> `ERR_UST_NOT_PAUSABLE`
- caller != pauser -> `ERR_UST_PAUSER_MISMATCH`
- pause while already paused -> `ERR_PAUSABLE_ENFORCED_PAUSE`
- unpause while not paused -> `ERR_PAUSABLE_EXPECTED_PAUSE`

### Frozen-state restrictions
When frozen, transfer/mint/burn paths are blocked with enforced-pause error.

---

## Solidity deployment examples

### Example 1: CREATE deployment

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract UST20Deployer {
    bytes4 constant UNIVERSAL_TOKEN_MAGIC = 0x45524320; // "ERC "

    function deployUST20(
        bytes32 name32,
        bytes32 symbol32,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) external returns (address token) {
        bytes memory initCode = bytes.concat(
            UNIVERSAL_TOKEN_MAGIC,
            abi.encode(name32, symbol32, decimals, initialSupply, minter, pauser)
        );

        assembly {
            token := create(0, add(initCode, 0x20), mload(initCode))
        }
        require(token != address(0), "UST20 deploy failed");
    }
}
```

Usage note:
- `bytes32("My Token")` / `bytes32("MTK")` already produce left-aligned, zero-padded values.

### Example 2: CREATE2 deployment

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract UST20Create2Deployer {
    bytes4 constant UNIVERSAL_TOKEN_MAGIC = 0x45524320;

    function deployUST20Create2(
        bytes32 salt,
        bytes32 name32,
        bytes32 symbol32,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) external returns (address token) {
        bytes memory initCode = bytes.concat(
            UNIVERSAL_TOKEN_MAGIC,
            abi.encode(name32, symbol32, decimals, initialSupply, minter, pauser)
        );

        assembly {
            token := create2(0, add(initCode, 0x20), mload(initCode), salt)
        }
        require(token != address(0), "UST20 create2 failed");
    }
}
```

---

## Rust deployment examples

### Example 1: build deployment payload

```rust
use fluentbase_sdk::universal_token::create_deployment_tx_with_roles;
use fluentbase_sdk::{Address, U256};

let init_code = create_deployment_tx_with_roles(
    "My Token",
    "MTK",
    18,
    U256::from(1_000_000u64),
    Some(Address::with_last_byte(0x01)), // minter
    Some(Address::with_last_byte(0x02)), // pauser
);

// `init_code` = magic prefix + ABI-encoded InitialSettings
```

### Example 2: deploy from a Fluentbase contract context

```rust
use fluentbase_sdk::universal_token::create_deployment_tx_with_roles;
use fluentbase_sdk::{Address, ExitCode, SharedAPI, U256};

fn deploy_ust20<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Address, ExitCode> {
    let init_code = create_deployment_tx_with_roles(
        "My Token",
        "MTK",
        18,
        U256::from(1_000_000u64),
        None,
        None,
    );

    let deployed = sdk.create(None, &U256::ZERO, &init_code).ok()?;
    Ok(Address::from_slice(deployed.as_ref()))
}

fn deploy_ust20_create2<SDK: SharedAPI>(sdk: &mut SDK, salt: U256) -> Result<Address, ExitCode> {
    let init_code = create_deployment_tx_with_roles(
        "My Token",
        "MTK",
        18,
        U256::from(1_000_000u64),
        None,
        None,
    );

    let deployed = sdk.create(Some(salt), &U256::ZERO, &init_code).ok()?;
    Ok(Address::from_slice(deployed.as_ref()))
}
```

---

## Storage model

UST20 stores:
- total supply,
- minter/pauser addresses,
- frozen flag,
- name/symbol/decimals,
- balances map,
- allowances nested map.

Slots are deterministic constants; mapping keys use SDK slot-derivation utilities.

---

## Compatibility notes

- legacy constructor compatibility still exists in prefetch helper paths,
- `token2022()` selector constant is reserved (not dispatched yet),
- universal token runtime is currently in delegated-runtime, system-runtime, and engine-metered sets.
