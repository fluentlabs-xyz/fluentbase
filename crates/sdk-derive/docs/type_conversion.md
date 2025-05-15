# Type Conversion in Fluentbase SDK

## Overview

Fluentbase SDK provides bidirectional conversion between Solidity and Rust types for smart contract interoperability. This conversion system powers macros like `router`, `client`, `derive_solidity_trait`, and `derive_solidity_client`.

## Type Mappings

### Solidity to Rust

| Solidity Type | Rust Type | Notes |
|--------------|-----------|-------|
| `bool` | `bool` | Direct mapping |
| `address` | `Address` | 20-byte type |
| `string` | `String` | UTF-8 encoded |
| `bytes` | `Bytes` | Dynamic byte array |
| `bytes1` to `bytes32` | `FixedBytes<N>` or `[u8; N]` | Fixed-size byte arrays |
| `uint8`/`uint16`/.../`uint128` | `u8`/`u16`/.../`u128` | Standard primitives |
| `uint256` | `U256` | 256-bit unsigned integer |
| `int8`/`int16`/.../`int128` | `i8`/`i16`/.../`i128` | Standard primitives |
| `int256` | `I256` | 256-bit signed integer |
| `T[]` | `Vec<T>` | Dynamic arrays |
| `T[N]` | `[T; N]` | Fixed-size arrays |
| `(T1,T2,...)` | `(T1,T2,...)` | Tuples |
| Structs | Rust structs | With `Codec` derive |

### Rust to Solidity

| Rust Type | Solidity Type | Notes |
|-----------|--------------|-------|
| `bool` | `bool` | Direct mapping |
| `Address` | `address` | 20-byte type |
| `String` or `&str` | `string` | UTF-8 encoded |
| `Bytes` | `bytes` | Dynamic byte array |
| `FixedBytes<N>` | `bytesN` | Fixed-size byte arrays |
| `[u8; N]` where N ≤ 32 | `bytesN` | Special case for byte arrays |
| `u8`/`u16`/.../`u128` | `uint8`/`uint16`/.../`uint128` | Standard integers |
| `U256` or `u256` | `uint256` | 256-bit unsigned integer |
| `i8`/`i16`/.../`i128` | `int8`/`int16`/.../`int128` | Signed integers |
| `I256` or `i256` | `int256` | 256-bit signed integer |
| `Vec<T>` | `T[]` | Dynamic arrays |
| `[T; N]` (except `[u8; N]` where N ≤ 32) | `T[N]` | Fixed-size arrays |
| `(T1,T2,...)` | `(T1,T2,...)` | Tuples |
| Custom structs | Tuples/structs | With `Codec` trait |

## Special Cases

- `[u8; N]` where N ≤ 32 is automatically converted to `bytesN` in Solidity
- References (`&T` and `&mut T`) are dereferenced during conversion
- Custom structs must implement `Codec` for serialization/deserialization

## Example

```rust, ignore
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::vec::Vec;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    Address,
    Bytes,
    SharedAPI,
    I256,
    U256,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait SolidityTypesAPI {
    // Test various Solidity types mapping to Rust types
    fn address_test(&self, addr: Address) -> Address;
    fn bytes_test(&self, data: Bytes) -> Bytes;
    fn fixed_bytes_test(&self, data: [u8; 32]) -> [u8; 32];
    fn uint256_test(&self, value: U256) -> U256;
    fn int256_test(&self, value: I256) -> I256;
    fn bool_test(&self, value: bool) -> bool;
    fn array_test(&self, values: Vec<U256>) -> Vec<U256>;
    fn multiple_params(&self, addr: Address, amount: U256, data: Bytes) -> bool;
    fn complex_return(&self, value: u64) -> (Address, U256, bool);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> SolidityTypesAPI for App<SDK> {
    #[function_id("addressTest(address)", validate(true))]
    fn address_test(&self, addr: Address) -> Address {
        addr
    }

    #[function_id("bytesTest(bytes)", validate(true))]
    fn bytes_test(&self, data: Bytes) -> Bytes {
        data
    }

    #[function_id("fixedBytesTest(bytes32)", validate(true))]
    fn fixed_bytes_test(&self, data: [u8; 32]) -> [u8; 32] {
        data
    }

    #[function_id("uint256Test(uint256)", validate(true))]
    fn uint256_test(&self, value: U256) -> U256 {
        value
    }

    #[function_id("int256Test(int256)", validate(true))]
    fn int256_test(&self, value: I256) -> I256 {
        value
    }

    #[function_id("boolTest(bool)", validate(true))]
    fn bool_test(&self, value: bool) -> bool {
        value
    }

    #[function_id("arrayTest(uint256[])", validate(true))]
    fn array_test(&self, values: Vec<U256>) -> Vec<U256> {
        values
    }

    #[function_id("multipleParams(address,uint256,bytes)", validate(true))]
    fn multiple_params(&self, addr: Address, amount: U256, data: Bytes) -> bool {
        !addr.is_zero() && !amount.is_zero() && !data.is_empty()
    }

    #[function_id("complexReturn(uint64)", validate(true))]
    fn complex_return(&self, value: u64) -> (Address, U256, bool) {
        (Address::default(), U256::from(value), true)
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(App);
```

## Best Practices

- Use standard numeric types when possible (`u8`, `u16`, etc.) rather than always using `U256`
- Implement `Codec` for all custom types used in contract interfaces
- For fixed-size byte arrays, use `[u8; N]` where N ≤ 32 for proper `bytesN` conversion
- Test your contract interfaces thoroughly to ensure type compatibility
- Only use `function_id` attribute inside `router` or `client` macros
- When working with complex data structures, consider gas costs in Solidity
