# Solidity Storage Macro

## Overview

The `solidity_storage` macro implements Solidity-compatible storage patterns in Fluentbase contracts. It automates the complex process of handling storage slots, key calculations, and value serialization/deserialization according to Solidity's storage layout rules. This enables your Rust contracts to maintain storage compatibility with Solidity contracts, facilitating seamless integration with existing Ethereum infrastructure.

## Usage

```rust,ignore
use fluentbase_sdk::{
    codec::Codec,
    derive::solidity_storage,
    Address, Bytes, U256
};

solidity_storage! {
    // Simple values
    Address Owner;                                    // Slot 0
    U256 Counter;                                     // Slot 1

    // Mappings
    mapping(Address => U256) Balance;                 // Slot 2
    mapping(Address => mapping(Address => U256)) Allowance;  // Slot 3

    // Arrays
    U256[] Values;                                    // Slot 4 (length), data at keccak256(4)
    Address[][] NestedAddresses;                      // Nested arrays

    // Custom structures (must implement Codec)
    MyStruct Data;                                    // Slot 5
    mapping(Address => MyStruct) StructMap;           // Slot 6
}
```

For each storage variable, the macro generates:

- A struct with the variable name
- A `SLOT` constant
- `get` and `set` methods
- Key calculation functions

### Supported Storage Types

- **Primitive Types**: `Address`, `U256`, `Bytes`, `bool`, etc.
- **Mappings**: Single-level and nested mappings (e.g., `mapping(K => V)`)
- **Arrays**: Dynamic arrays of any type, including multi-dimensional arrays
- **Custom Structures**: User-defined types that implement the `Codec` trait

## Examples

### Simple Storage Example

```rust,ignore
use fluentbase_sdk::{
    codec::Codec,
    derive::solidity_storage,
    U256
};

#[derive(Contract)]
struct SimpleStorage<SDK> {
    sdk: SDK,
}

solidity_storage! {
    U256 StoredValue;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> SimpleStorage<SDK> {
    #[function_id("store(uint256)")]
    pub fn store(&mut self, value: U256) {
        StoredValue::set(&mut self.sdk, value);
    }

    #[function_id("retrieve()")]
    pub fn retrieve(&self) -> U256 {
        StoredValue::get(&self.sdk)
    }
}
```

### Complex Types Example

```rust,ignore
use fluentbase_sdk::{
    codec::Codec,
    derive::solidity_storage,
    Address, Bytes, U256
};

#[derive(Codec, Debug, Default, Clone, PartialEq)]
pub struct Proposal {
    pub proposer: Address,
    pub description: Bytes,
    pub votes_for: U256,
    pub votes_against: U256,
}

solidity_storage! {
    // Track proposals by ID
    mapping(U256 => Proposal) Proposals;

    // Track votes by address
    mapping(Address => mapping(U256 => bool)) HasVoted;

    // Array of active proposals
    U256[] ActiveProposalIds;
}

impl<SDK: SharedAPI> GovernanceContract<SDK> {
    pub fn create_proposal(&mut self, proposer: Address, description: Bytes) -> U256 {
        let proposal_id = self.next_proposal_id();

        let proposal = Proposal {
            proposer,
            description,
            votes_for: U256::zero(),
            votes_against: U256::zero(),
        };

        Proposals::set(&mut self.sdk, proposal_id, proposal);

        // Add to active proposals
        let new_index = ActiveProposalIds::get_length(&self.sdk);
        ActiveProposalIds::set_at(&mut self.sdk, new_index, proposal_id);

        proposal_id
    }

    pub fn vote(&mut self, voter: Address, proposal_id: U256, vote_for: bool) {
        // Check hasn't voted
        let has_voted = HasVoted::get(&self.sdk, voter, proposal_id);
        if has_voted {
            return;
        }

        // Get proposal
        let mut proposal = Proposals::get(&self.sdk, proposal_id);

        // Update vote count
        if vote_for {
            proposal.votes_for += U256::one();
        } else {
            proposal.votes_against += U256::one();
        }

        // Store updated proposal
        Proposals::set(&mut self.sdk, proposal_id, proposal);

        // Mark as voted
        HasVoted::set(&mut self.sdk, voter, proposal_id, true);
    }
}
```

## Notes & Best Practices

- **Slot Assignment**: Storage slots are assigned sequentially starting from 0
- **Type Requirements**: All custom types must implement the `Codec` trait
- **Key Calculations**: The macro follows Solidity's standard algorithm for key calculations:
  - Simple values are stored directly at their slot
  - Mapping keys use keccak256(key . slot) to determine storage position
  - Array lengths are stored at the slot, with data starting at keccak256(slot)
- **Performance**: Access mappings and arrays efficiently, as each level of nesting requires additional hash calculations
- **Consistency**: Use the same storage layout in related contracts to ensure compatibility
- **Method Naming**: For each variable, the following methods are generated:
  - `get(sdk, ...)` - Retrieve a value
  - `set(sdk, ..., value)` - Store a value
  - For arrays, additional methods like `get_length` and `set_at` are available
- **Storage Collisions**: Be careful with manual slot calculations that might conflict with the macro's slot assignments
- **Gas Usage**: Complex storage operations (especially nested mappings/arrays) can be gas-intensive
