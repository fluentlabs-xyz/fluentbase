# Constructor Macro

## Overview

The `constructor` macro provides a dedicated way to define contract initialization logic. It generates a `deploy()`
entry point that handles parameter decoding during contract deployment, without the overhead of full method routing.

## Usage

```rust,ignore
#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> MyContract<SDK> {
    pub fn constructor(&mut self, owner: Address, initial_supply: U256) {
        // Initialization logic
    }
}
```

### Attributes

- **mode**: Encoding mode - `"solidity"` (EVM compatible) or `"fluent"` (optimized)

## Examples

### Simple Constructor

```rust,ignore
#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> SimpleContract<SDK> {
    pub fn constructor(&mut self) {
        // Initialize without parameters
    }
}
```

### Constructor with Parameters

```rust,ignore
#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> Token<SDK> {
    pub fn constructor(&mut self, name: String, symbol: String, decimals: u8) {
        // Store initialization parameters
    }
}
```

### With Trait Implementation

```rust,ignore
#[derive(Contract)]
pub struct ERC20<SDK> {
    sdk: SDK,
}

pub trait ERC20Interface {
    fn balance_of(&self, owner: Address) -> U256;
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
}

// Separate constructor for initialization
#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn constructor(&mut self, name: String, symbol: String, initial_supply: U256) {
        // Initialize token
    }
}

// Router for trait methods
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ERC20Interface for ERC20<SDK> {
    fn balance_of(&self, owner: Address) -> U256 {
        // Implementation
    }

    fn transfer(&mut self, to: Address, amount: U256) -> bool {
        // Implementation
    }
}
```

### Complete Contract Example

```rust,ignore
#[derive(Contract)]
pub struct Token<SDK> {
    sdk: SDK,
}

// Initialization logic
#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> Token<SDK> {
    pub fn constructor(&mut self, initial_supply: U256) {
        // Set initial supply
    }
}

// Runtime methods
#[router(mode = "solidity")]
impl<SDK: SharedAPI> Token<SDK> {
    pub fn balance_of(&self, account: Address) -> U256 {
        // Query balance
    }

    pub fn transfer(&mut self, to: Address, amount: U256) -> bool {
        // Transfer logic
    }
}
```

## When to Use

### Use `#[constructor]` when

- Working with trait implementations (traits can't have constructors)
- You want clear separation between initialization and runtime logic
- Your contract has complex initialization but simple runtime methods
- You prefer explicit, focused macros

### Use constructor in `#[router]` when

- You want everything in one place
- Your contract is simple with few methods
- Not using trait implementations
- You prefer minimal macro usage

## Generated Code

The macro generates:

1. **Codec Types**:
    - `ConstructorCall` - wrapper for constructor arguments
    - `ConstructorCallArgs` - tuple of actual parameter types
    - Encoding/decoding implementations

2. **Deploy Method**:

   ```rust,ignore
   pub fn deploy(&mut self) {
       // Reads input data
       // Decodes constructor parameters
       // Calls constructor with decoded arguments
   }
   ```

## Requirements

- **Method Name**: Must be exactly `constructor`
- **Single Method**: The impl block must contain exactly one constructor method
- **Visibility**: Constructor should be `pub`
- **Parameters**: Supports any number of parameters (including zero)

## Notes

- **Other Methods**: Any other methods in the block are ignored with a warning
- **No Selectors**: Constructor has no function selector (only called during deployment)
- **Compatible**: Works seamlessly with `#[router]` for complete contracts
- **Codec Reuse**: Uses the same codec infrastructure as `#[router]`
- **Mode Consistency**: Use the same mode for both `#[constructor]` and `#[router]`

## Error Handling

The macro will fail compilation if:

- No `constructor` method is found
- Multiple `constructor` methods are defined
- The impl block contains syntax errors

Warnings are emitted for:

- Non-constructor methods in the impl block (they are ignored)
