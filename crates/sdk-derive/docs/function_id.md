# Function ID Attribute

## Overview

Specifies custom function selectors for smart contract methods. This attribute macro allows overriding the default function selector (first 4 bytes of keccak256 hash) with a custom value.

## Selector Formats

Three ways to specify a function selector:

1. **Solidity Signature** (will be hashed):

```rust
#[function_id("transfer(address,uint256)")]
fn transfer(&mut self, to: Address, amount: U256)
```

2. **Hex Format**:

```rust
#[function_id("0xa9059cbb")]
fn transfer(&mut self, to: Address, amount: U256)
```

3. **Raw Bytes**:

```rust
#[function_id([169, 5, 156, 187])]
fn transfer(&mut self, to: Address, amount: U256)
```

## Function Name Convention

When using signature format, function names are automatically converted to camelCase:

- `transfer_all` → `transferAll`
- `get_balance` → `getBalance`

## Signature Validation

By default, the macro validates that provided signatures match function parameters.
Validation can be disabled:

```rust
#[function_id("transfer(address,uint256)", validate(false))]
fn transfer(&mut self, to: Address, amount: U256)
```

## Error Cases

- Invalid selector format provided
- Signature validation fails (when enabled)
- Selector length is not 4 bytes
- Parameter type mismatches in signature

See the router macro documentation for details on function dispatching and selector usage.
