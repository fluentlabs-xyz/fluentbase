# fluentbase-sdk-derive

Procedural macros for simplifying smart contract development in the Fluentbase ecosystem.

## Features

- **Router Generation**: Automatic function dispatch and parameter handling
- **Storage Layout**: Solidity-compatible state variable management
- **Client Generation**: Type-safe contract interaction wrappers
- **Function Identifiers**: Custom function selector support

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
fluentbase-sdk-derive = "0.1.0"
```

Basic example:

```rust
use fluentbase_sdk_derive::{router, client, function_id, solidity_storage};

// Define contract storage
solidity_storage! {
    mapping(Address => U256) Balance;
    Address Owner;
}

// Define contract API
#[client(mode = "solidity")]
trait TokenAPI {
    #[function_id("transfer(address,uint256)")]
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
}

// Implement contract logic
#[router(mode = "solidity")]
impl<SDK: SharedAPI> Contract<SDK> {
    pub fn transfer(&mut self, to: Address, amount: U256) -> bool {
        // Contract logic here
    }
}
```

## Documentation

Detailed documentation for each macro:

- [Router](docs/router.md) - Function dispatch and parameter handling
- [Storage](docs/storage.md) - State variable management
- [Client](docs/client.md) - Contract interaction
- [Function ID](docs/function_id.md) - Function selectors

## License

[MIT](LICENSE)
