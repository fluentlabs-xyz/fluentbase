# Client Macro

## Overview

Generates type-safe client-side wrappers for interacting with smart contracts. This macro creates a client struct with methods for encoding parameters, making contract calls, and decoding results.

## Usage

Define your contract interface as a trait:

```rust
#[client(mode = "solidity")]
trait TokenAPI {
    #[function_id("transfer(address,uint256)")]
    fn transfer(&mut self, to: Address, amount: U256) -> bool;

    #[function_id("balanceOf(address)")]
    fn balance_of(&mut self, owner: Address) -> U256;
}
```

The macro generates a client struct named `{TraitName}Client`:

```rust
let client = TokenAPIClient::new(sdk);
let result = client.transfer(
    contract_address,
    value,
    gas_limit,
    recipient,
    amount
);
```

## Encoding Modes

- `"solidity"`: Big-endian, 32-byte aligned (Ethereum ABI compatible)
- `"fluent"`: Little-endian, 4-byte aligned (compact representation)

## Generated Features

For each trait method, the client generates:

- Parameter encoding: `encode_{method_name}`
- Result decoding: `decode_{method_name}`
- Contract call wrapper with:
  - Gas limit management
  - Value transfer handling
  - Safety checks

## Safety Checks

Generated code includes runtime checks for:

- Sufficient funds for transaction
- Adequate gas limit
- Contract call success (exit code)

## Error Cases

- Insufficient funds for transaction
- Insufficient gas limit
- Contract call execution failure
- Parameter encoding/decoding errors

See the router macro documentation for details on function selectors and encoding modes.
