# Router Macro

## Overview

The `router` macro provides a robust method dispatch system for Fluentbase smart contracts. It automatically transforms
function calls with Solidity-compatible selectors into appropriate Rust function calls, handling parameter decoding and
result encoding. The router macro serves as the foundation for building interoperable smart contracts that can be called
from EVM-compatible environments.

## Usage

```rust,ignore
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ContractTrait for Contract<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&self, message: String) -> String {
        message
    }

    #[function_id("0xe8927fbc")]  // Direct selector specification
    fn custom_method(&self, value: U256) -> bool {
        // Implementation
    }

    // Special method - handles unmatched selectors
    fn fallback(&self) {
        // Fallback logic
    }
}
```

### Attributes

- **mode**: Specifies encoding mode - either `"solidity"` (full EVM compatibility) or `"fluent"` (optimized format)

### Function ID Specification

- **Solidity Signature**: `#[function_id("transfer(address,uint256)")]`
- **Direct Hex**: `#[function_id("0xa9059cbb")]`
- **Raw Bytes**: `#[function_id([169, 5, 156, 187])]`
- **Validation**: Control selector validation with `#[function_id("...", validate(false))]`

## Examples

### Trait Implementation

```rust,ignore
#[derive(Contract)]
pub struct ERC20<SDK> {
    sdk: SDK,
}

pub trait ERC20Interface {
    fn total_supply(&self) -> U256;
    fn balance_of(&self, owner: Address) -> U256;
    fn transfer(&self, to: Address, amount: U256) -> bool;
}

// Separate constructor for initialization
#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn constructor(&mut self, name: String, symbol: String, initial_supply: U256) {
        // Initialize token parameters
    }
}

// Router for trait implementation
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ERC20Interface for ERC20<SDK> {
    fn total_supply(&self) -> U256 {
        // Implementation
    }

    fn balance_of(&self, owner: Address) -> U256 {
        // Implementation
    }

    fn transfer(&self, to: Address, amount: U256) -> bool {
        // Implementation
    }
}
```

### Direct Implementation with Constructor

```rust,ignore
#[derive(Contract)]
pub struct SimpleStorage<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> SimpleStorage<SDK> {
    // Constructor - generates deploy() entry point
    pub fn constructor(&mut self, initial_value: U256) {
        // Initialization logic
    }

    pub fn store(&mut self, value: U256) {
        // Implementation
    }

    pub fn retrieve(&self) -> U256 {
        // Implementation
    }

    // Not exposed via selector (private method)
    fn internal_method(&self) {
        // Implementation
    }
}
```

## Special Methods

### Constructor

The `constructor` method is a special initialization function:

- **Name**: Must be exactly `constructor`
- **Purpose**: Runs once during contract deployment
- **Generated**: Creates a `deploy()` entry point that decodes parameters
- **No selector**: Not callable after deployment
- **Alternative**: Use `#[constructor]` macro for initialization-only implementations

```rust,ignore
pub fn constructor(&mut self, owner: Address, initial_supply: U256) {
    // Initialization code
}
```

### Fallback

The `fallback` method handles unmatched function selectors:

- **Signature**: Must be `fn fallback(&self)` with no parameters or return
- **Optional**: Contract reverts on unknown selectors if not defined

## Notes & Best Practices

- **Method Visibility**: In direct implementations, only `pub` methods are included in selector routing
- **Constructor in Router**: Can include constructor in direct implementations for convenience
- **Separate Constructor**: Use `#[constructor]` macro when you only need initialization
- **Error Handling**: Invalid selectors trigger the fallback handler or panic if none is defined
- **Generated Code**: Includes method-specific codec implementations and dispatch logic
- **Performance**: The `"fluent"` mode provides more compact encoding at the cost of EVM compatibility
- **Function IDs**: Always use validation to ensure selector consistency
- **Selector Collisions**: Automatically detected at compile time
