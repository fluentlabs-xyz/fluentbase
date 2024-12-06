# Storage Macro

## Overview

Implements Solidity's storage patterns for smart contract state variables. This macro handles slot management, key calculations, and generates type-safe storage access methods.

## Storage Layout

Contract storage is organized as a persistent key-value store where each slot is 32 bytes (256 bits):

- Simple types are stored sequentially from slot 0
- Mapping slots store no value but are used for key calculations
- Dynamic array slots store length, with data at `keccak256(slot)`
- Nested structures are stored contiguously from their slot

## Usage

Define your contract storage layout:

```rust
solidity_storage! {
    // Simple values
    Address Owner;      // Slot 0
    U256 Balance;      // Slot 1

    // Mappings
    mapping(Address => U256) Balances;
    mapping(Address => mapping(Address => U256)) Allowances;

    // Arrays
    U256[] Values;
    Address[][][] NestedArray;

    // Custom structures
    UserData Data;
    mapping(Address => UserData) UserMap;
}
```

## Storage Types

### Simple Values

```rust
Address Owner;     // Single slot storage
U256 Balance;      // Numerical values
Bytes Data;       // Dynamic data
```

### Mappings

```rust
mapping(K => V) Map;                    // Single mapping
mapping(K => mapping(K2 => V)) Nested;  // Nested mapping
```

### Arrays

```rust
T[] Array;              // Dynamic array
T[][][] NestedArray;    // Multi-dimensional
```

### Custom Structures

```rust
#[derive(Codec)]
struct UserData {
    balance: U256,
    active: bool,
}

solidity_storage! {
    UserData Data;
    mapping(Address => UserData) UserMap;
}
```

## Generated Features

For each storage variable, the macro generates:

### Storage Slot

```rust
const SLOT: U256 = U256::from_limbs([slot_index, 0, 0, 0]);
```

### Access Methods

```rust
// Key calculation
fn key<SDK: SharedAPI>(sdk: &SDK, ...args) -> U256;

// Getter/setter methods
fn get<SDK: SharedAPI>(sdk: &SDK, ...args) -> T;
fn set<SDK: SharedAPI>(sdk: &mut SDK, ...args, value: T);
```

## Type Mappings

| Solidity Type | Rust Type     | Storage Layout |
| ------------- | ------------- | -------------- |
| uint256       | U256          | Single slot    |
| address       | Address       | Single slot    |
| bytes         | Bytes         | Single slot    |
| mapping       | -             | Hashed slots   |
| array         | -             | Sequential     |
| struct        | Custom type*  | Multiple slots |

\* Custom types must implement `Codec` trait

## Key Features

- Automatic sequential slot assignment
- Type-safe Solidity compatibility
- Standard storage layout algorithms
- Optimized getter/setter methods
- Support for complex types (mappings, arrays, structs)

## Error Cases

- Invalid storage patterns
- Unsupported types
- Missing trait implementations
- Storage access failures
- Key calculation errors
