# Client Macro

## Overview

The `client` macro generates type-safe client code for interacting with Fluentbase smart contracts. It creates a client struct and implements methods that match your trait definition, handling parameter encoding, contract calls, and result decoding automatically. This enables seamless interaction with contracts while maintaining type safety and consistency with the contract interface.

## Usage

```rust,ignore
#[client(mode = "solidity")]
trait TokenInterface {
    #[function_id("balanceOf(address)")]
    fn balance_of(&self, owner: Address) -> U256;

    #[function_id("transfer(address,uint256)")]
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
}
```

The macro generates a client struct (named `TraitNameClient`, e.g., `TokenInterfaceClient`) with implementing methods that handle:

- Function selector generation
- Parameter encoding
- Gas and value management
- Contract call execution
- Result decoding

### Attributes

- **mode**: Specifies encoding mode - either `"solidity"` (full EVM compatibility) or `"fluent"` (optimized format)

### Function ID Specification

Same options as the router macro:

- **Solidity Signature**: `#[function_id("transfer(address,uint256)")]`
- **Direct Hex**: `#[function_id("0xa9059cbb")]`
- **Raw Bytes**: `#[function_id([169, 5, 156, 187])]`
- **Validation**: Control selector validation with `#[function_id("...", validate(false))]`

## Examples

### Basic ERC20 Client

```rust,ignore
#[client(mode = "solidity")]
trait ERC20 {
    #[function_id("balanceOf(address)")]
    fn balance_of(&self, owner: Address) -> U256;

    #[function_id("transfer(address,uint256)")]
    fn transfer(&mut self, to: Address, amount: U256) -> bool;

    #[function_id("approve(address,uint256)")]
    fn approve(&mut self, spender: Address, amount: U256) -> bool;
}

// Use the generated client
impl<SDK: SharedAPI> ERC20Client<SDK> {
    // Add custom convenience methods
    pub fn check_and_transfer(
        &mut self,
        contract_address: Address,
        to: Address,
        amount: U256,
        gas_limit: u64
    ) -> bool {
        let balance = self.balance_of(contract_address, U256::zero(), gas_limit, to);
        if balance >= amount {
            self.transfer(contract_address, U256::zero(), gas_limit, to, amount)
        } else {
            false
        }
    }
}
```

### Client with Complex Types

```rust,ignore
#[derive(Codec, Debug)]
pub struct VotingConfig {
    pub threshold: U256,
    pub voting_period: u64,
}

#[client(mode = "solidity")]
trait Governance {
    #[function_id("propose(string,address[],uint256)")]
    fn propose(&mut self, description: String, targets: Vec<Address>, value: U256) -> U256;

    #[function_id("getConfig()")]
    fn get_config(&self) -> VotingConfig;
}
```

## Notes & Best Practices

- **Automatic Client Methods**: For each trait method, the macro generates a client method that takes contract address, value, gas limit, and function parameters
- **Return Types**: Return types are automatically decoded from contract call results
- **Error Handling**: Client methods panic if the contract call fails (for simplicity - you may want to extend with custom error handling)
- **SDK Requirement**: The generated client requires an SDK type that implements `fluentbase_sdk::SharedAPI`
- **Trait Methods**: Only trait methods are included in the client (not custom implementations)
- **Method Receivers**: The macro respects method receivers - `&self` generates a read-only call, `&mut self` allows value transfer
- **ABI Compatibility**: Use the same encoding mode (`solidity` or `fluent`) as the contract you're calling
- **Type Consistency**: Ensure parameter and return types match between client and contract for proper encoding/decoding
