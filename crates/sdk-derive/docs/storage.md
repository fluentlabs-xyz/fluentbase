# Storage Layout

The `#[derive(Storage)]` macro implements Solidity-compatible storage layout for Fluentbase contracts, automatically
handling slot allocation and data packing according
to [Solidity's storage layout rules](https://docs.soliditylang.org/en/latest/internals/layout_in_storage.html).

## Basic Usage

```rust,ignore
use fluentbase_sdk::{derive::Storage, storage::*};

#[derive(Storage)]
pub struct MyContract<SDK> {
    sdk: SDK,
    owner: StorageAddress,
    counter: StorageU256,
    is_active: StorageBool,
    balances: StorageMap<Address, StorageU256>,
}
```

## Storage Types

### Primitive Types

```rust,ignore
// Unsigned integers
StorageU8, StorageU16, StorageU32, StorageU64, StorageU128, StorageU256

// Signed integers  
StorageI8, StorageI16, StorageI32, StorageI64, StorageI128

// Other primitives
StorageBool              // 1 byte
StorageAddress           // 20 bytes
StorageBytes32           // 32 bytes
```

### Complex Types

```rust,ignore
// Mappings
StorageMap<K, V>         // Key-value storage

// Arrays
StorageArray<T, N>       // Fixed-size array
StorageVec<T>           // Dynamic array

// Strings and bytes
StorageString           // Dynamic UTF-8 string
StorageBytes            // Dynamic byte array
```

## Working with Storage

Each field gets an accessor method that returns the appropriate interface:

```rust,ignore
impl<SDK: SharedAPI> MyContract<SDK> {
    pub fn increment(&mut self) {
        // Read current value
        let current = self.counter_accessor().get(&self.sdk);
        
        // Write new value
        self.counter_accessor().set(&mut self.sdk, current + U256::from(1));
    }
    
    pub fn set_balance(&mut self, user: Address, amount: U256) {
        self.balances_accessor()
            .entry(user)
            .set(&mut self.sdk, amount);
    }
}
```

## Automatic Packing

Types smaller than 32 bytes are automatically packed into storage slots:

```rust,ignore
#[derive(Storage)]
pub struct PackedData {
    is_active: StorageBool,     // Slot 0, offset 31
    is_paused: StorageBool,     // Slot 0, offset 30  
    version: StorageU32,         // Slot 0, offset 26
    owner: StorageAddress,       // Slot 0, offset 6
    counter: StorageU256,        // Slot 1, offset 0
}
```

## Nested Structures

Create composable storage structures:

```rust,ignore
#[derive(Storage)]
pub struct Config {
    version: StorageU32,
    max_supply: StorageU256,
}

#[derive(Storage)]
pub struct Game<SDK> {
    sdk: SDK,
    settings: Config,
    players: StorageMap<Address, Config>,
}
```

## Storage Slot Calculation

The system follows Solidity's conventions:

- **Simple values**: Stored sequentially starting from slot 0
- **Mappings**: `slot = keccak256(key || base_slot)`
- **Dynamic arrays**: Length at base slot, elements at `keccak256(base_slot) + index`
- **Strings/bytes < 32 bytes**: Data and length in base slot
- **Strings/bytes ≥ 32 bytes**: Length in base slot, data at `keccak256(base_slot)`

## Complete Example

```rust,ignore
#[derive(Storage)]
pub struct ERC20<SDK> {
    sdk: SDK,
    name: StorageString,
    symbol: StorageString,
    total_supply: StorageU256,
    balances: StorageMap<Address, StorageU256>,
    allowances: StorageMap<Address, StorageMap<Address, StorageU256>>,
}

impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn transfer(&mut self, to: Address, amount: U256) {
        let from = self.sdk.context().contract_caller();
        
        // Get current balances
        let from_balance = self.balances_accessor().entry(from).get(&self.sdk);
        let to_balance = self.balances_accessor().entry(to).get(&self.sdk);
        
        // Check and update
        assert!(from_balance >= amount, "insufficient balance");
        
        self.balances_accessor()
            .entry(from)
            .set(&mut self.sdk, from_balance - amount);
            
        self.balances_accessor()
            .entry(to)
            .set(&mut self.sdk, to_balance + amount);
    }
}
```

## See Also

For working examples, check
the [Fluentbase examples repository](https://github.com/fluentlabs-xyz/fluentbase/tree/devel/examples).