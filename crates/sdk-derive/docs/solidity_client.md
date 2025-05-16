# Solidity Client Macro

## Overview

The `derive_solidity_client` macro generates a Rust client interface directly from Solidity contract or interface definitions. This macro combines the functionality of `derive_solidity_trait` and `client` macros, creating a complete client implementation for interacting with Solidity contracts.

## Usage

Like `derive_solidity_trait`, this macro supports two ways of specifying Solidity interfaces:

### 1. From a file path

```rust,ignore
derive_solidity_client!("abi/IRouterAPI.sol");
```

### 2. Directly inline

```rust,ignore
derive_solidity_client!(
    interface IRouterApi {
        function greeting(string calldata message) external view returns (string calldata return_0);
        function customGreeting(string calldata message) external view returns (string calldata return_0);
    }
);
```

## Examples

### Basic Usage

```rust,ignore
use fluentbase_sdk::{
    derive::derive_solidity_client,
    Address, U256, SharedAPI
};

// Define the client from inline Solidity
derive_solidity_client!(
    interface IERC20 {
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
    }
);

// Use the generated client in your code
fn interact_with_erc20<SDK: SharedAPI>(sdk: SDK, token_address: Address) {
    let mut client = IERC20Client::new(sdk);

    // Call the contract methods
    let total = client.total_supply(token_address, U256::zero(), 100000);
    let balance = client.balance_of(token_address, U256::zero(), 100000, my_address);

    // Send tokens
    let success = client.transfer(
        token_address,
        U256::zero(), // No value sent with the call
        100000,       // Gas limit
        recipient,    // To address
        U256::from(100) // Amount to transfer
    );
}
```

### Using with File Path

```rust,ignore
use fluentbase_sdk::derive::derive_solidity_client;

// Load from a file
derive_solidity_client!("abi/IToken.sol");

// The client is generated automatically
// You can use it like this:
let mut client = ITokenClient::new(sdk);
let result = client.method_name(contract_address, value, gas_limit, ...args);
```

## Notes & Best Practices

- **Parameter Order**: Generated client methods take standard parameters first:
  - `contract_address`: The address of the contract to call
  - `value`: Amount of native tokens to send with the call (usually `U256::zero()`)
  - `gas_limit`: Maximum gas for the transaction
  - Then any function-specific parameters

- **Client Structure**: The generated client follows the same patterns as the [`client` macro](client.md)

- **Trait Generation**: The macro first generates a trait (as `derive_solidity_trait` would), then applies `#[client(mode = "solidity")]` to it

- **Best Practice**: For most use cases, this is simpler than using `derive_solidity_trait` and `client` separately
