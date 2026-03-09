# Create2Factory Contract

`Create2Factory` is a native Fluentbase system contract for deploying child contracts via `CREATE` and `CREATE2` from a shared precompile runtime.

---

## Address

The system precompile address is:

- `PRECOMPILE_CREATE2_FACTORY = 0x0000000000000000000000000000000000520011`

This constant is defined in `crates/types/src/genesis.rs`.

---

## What it does

The contract exposes 3 methods:

1. `deployCreate(bytes init_code) returns (address)`
2. `deployCreate2(uint256 salt, bytes init_code) returns (address)`
3. `computeCreate2Address(uint256 salt, bytes32 init_code_hash) returns (address)`

---

## How deployment works

- `deployCreate` forwards deployment to native SDK `create(None, ...)`, which now routes to `SYSCALL_ID_CREATE`.
- `deployCreate2` forwards deployment to native SDK `create(Some(salt), ...)`, which routes to `SYSCALL_ID_CREATE2`.

On success, factory returns deployed contract address.

---

## CREATE2 address prediction

`computeCreate2Address` uses Fluentbase helper `calc_create2_address` and computes:

```text
keccak256(0xff ++ factory_address ++ salt ++ init_code_hash)[12:]
```

This should match the address returned by `deployCreate2` when called with the same `salt` and `init_code`.

---

## Solidity call examples

### Deploy with CREATE

```solidity
bytes memory initCode = hex"...";
address deployed = ICreate2Factory(0x0000000000000000000000000000000000520011)
    .deployCreate(initCode);
```

### Deploy with CREATE2

```solidity
uint256 salt = 123;
bytes memory initCode = hex"...";
address deployed = ICreate2Factory(0x0000000000000000000000000000000000520011)
    .deployCreate2(salt, initCode);
```

### Predict CREATE2 address

```solidity
uint256 salt = 123;
bytes32 initCodeHash = keccak256(initCode);
address predicted = ICreate2Factory(0x0000000000000000000000000000000000520011)
    .computeCreate2Address(salt, initCodeHash);
```

---

## Notes

- `deployCreate2` will fail if the computed address is already occupied.
- `init_code` must be valid for the target runtime/path you deploy through.
- `computeCreate2Address` is pure deterministic math; it does not check chain state.
