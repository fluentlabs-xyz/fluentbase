# Solidity Trait Macro

## Overview

The `derive_solidity_trait` macro generates Rust traits directly from Solidity contract or interface definitions. This enables seamless translation of Solidity interfaces to Rust, preserving function signatures, parameter types, and return types.

## Usage

The macro supports two ways of specifying Solidity interfaces:

### 1. From a file path

```rust,ignore
derive_solidity_trait!("abi/IRouterAPI.sol");
```

### 2. Directly inline

```rust,ignore
derive_solidity_trait!(
    interface IRouterApi {
        function greeting(string calldata message) external view returns (string calldata return_0);
        function customGreeting(string calldata message) external view returns (string calldata return_0);
    }
);
```

## Examples

### Using Inline Solidity

```rust,ignore
use fluentbase_sdk::{
    derive::derive_solidity_trait,
    Address, U256
};

derive_solidity_trait!(
    interface IERC20 {
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
    }
);

// Implement the trait
#[router(mode = "solidity")]
impl<SDK: SharedAPI> IERC20 for MyToken<SDK> {
    fn total_supply(&self) -> U256 {
        // Implementation
    }

    fn balance_of(&self, account: Address) -> U256 {
        // Implementation
    }

    fn transfer(&mut self, to: Address, amount: U256) -> bool {
        // Implementation
    }
}
```

### Using File Path

```rust,ignore
use fluentbase_sdk::derive::derive_solidity_trait;

// File: abi/IGovernor.sol
// interface IGovernor {
//     struct ProposalVote {
//         uint256 againstVotes;
//         uint256 forVotes;
//         uint256 abstainVotes;
//     }
//
//     function propose(...) external returns (uint256);
//     function castVote(...) external returns (uint256);
//     ...
// }

derive_solidity_trait!("abi/IGovernor.sol");

// Use the generated trait in your contract
#[router(mode = "solidity")]
impl<SDK: SharedAPI> IGovernor for MyGovernor<SDK> {
    // Implement methods here
}
```

## Notes & Best Practices

- **Function Names**: Solidity-style camelCase names are converted to Rust-style snake_case
- **Method Receivers**: View/pure functions get `&self`, other functions get `&mut self`
- **Integration**: Works seamlessly with the `router` macro for implementing contracts
- **Client Generation**: For generating client code from the same interface, see [Solidity Client Macro](solidity_client.md)
- **Type Mappings**: Basic Solidity types are automatically mapped to their Rust equivalents
- **Best Practices**: Keep your Solidity interfaces in separate files for better organization
