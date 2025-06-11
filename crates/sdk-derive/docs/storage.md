# Solidity Storage Macro

Implements Solidity-compatible storage in Fluentbase contracts following [Solidity's storage layout specification](https://docs.soliditylang.org/en/latest/internals/layout_in_storage.html). Provides significant code size reduction through direct storage access for primitive types.

## Usage

```rust,ignore
solidity_storage! {
    // Simple values
    Address Owner;                                    // Slot 0
    bool Paused;                                      // Slot 1

    // Mappings
    mapping(Address => U256) Balance;                 // Slot 2
    mapping(Address => mapping(Address => U256)) Allowance;  // Slot 3

    // Arrays
    U256[] Values;                                    // Slot 4

    // Custom types (must implement Codec)
    MyStruct Data;                                    // Slot 5
}
```

## Storage Key Calculation

- **Simple variables**: `key = slot`
- **Mappings**: `key = keccak256(h(k) . p)` where `h(k)` is the padded key and `p` is the slot
- **Nested mappings**: `key = keccak256(h(k2) . keccak256(h(k1) . p))`
- **Arrays**: Element at index `i` is at `keccak256(slot) + i`

## Direct Storage vs Encoding/Decoding

The macro automatically selects the most efficient storage method:

- **`DirectStorage`**: For types ≤ 32 bytes (integers, booleans, addresses, small byte arrays)
- **`StorageValueSolidity`**: For complex types requiring serialization

## Generated API

For each variable `Name`, the macro generates:

- A struct `Name` with `SLOT` constant
- Methods `get(sdk, ...)` and `set(sdk, ..., value)`

For examples, see the [fluentbase examples repository](https://github.com/fluentlabs-xyz/fluentbase/tree/devel/examples/storage).
