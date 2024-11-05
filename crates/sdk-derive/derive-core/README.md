# SDK Derive Macros

This documentation covers two main macros for building Solidity-compatible smart contracts in Rust: `router` and `client`. These macros enable seamless interaction with Solidity-style contracts in a no_std environment.

- [SDK Derive Macros](#sdk-derive-macros)
  - [Router Macro](#router-macro)
    - [Key Features](#key-features)
    - [Encoding Modes](#encoding-modes)
    - [Function ID Definition and Validation](#function-id-definition-and-validation)
      - [Specifying Function IDs](#specifying-function-ids)
      - [Validation System](#validation-system)
        - [How Validation Works](#how-validation-works)
        - [Disabling Validation](#disabling-validation)
        - [Error Messages](#error-messages)
    - [Method Visibility](#method-visibility)
      - [Trait Implementation](#trait-implementation)
      - [Direct Implementation](#direct-implementation)
    - [Special Methods](#special-methods)
    - [Testing](#testing)
    - [Arguments and Types](#arguments-and-types)
      - [Type Mappings](#type-mappings)
      - [Basic Usage](#basic-usage)
      - [Using Structures](#using-structures)
  - [Client Macro](#client-macro)
    - [Key Features](#key-features-1)
    - [Encoding Modes](#encoding-modes-1)
    - [Example](#example)
    - [Common Features for Both Macros](#common-features-for-both-macros)
  - [Storage Macro](#storage-macro)
    - [Understanding Solidity Storage](#understanding-solidity-storage)
    - [Storage Macro Key Features](#storage-macro-key-features)
    - [Supported Storage Types](#supported-storage-types)
    - [Type Requirements](#type-requirements)
    - [Complete Example](#complete-example)
    - [Testing Storage Layout](#testing-storage-layout)
    - [Generated Methods](#generated-methods)
    - [Type Mappings](#type-mappings-1)
    - [Notes](#notes)

## Router Macro

The `router` macro provides a streamlined way to handle Solidity-compatible contract interactions. In Fluentbase, all contract interactions go through a `main` function that serves as the primary entry point. Each transaction includes:

- A function selector (first 4 bytes of the keccak256 hash of the function signature)
- Function arguments (formatted according to the Solidity ABI standard)

When a transaction is received:

1. The `main` function is triggered
2. Input data is decoded
3. Call is routed to the appropriate function
4. Result is encoded and returned

The `router` macro automates this entire process, generating all necessary boilerplate code for the dispatch mechanism.

```rust
#[derive(Contract)]
struct ROUTER<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: String) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&self, message: String) -> String {
        message
    }
}

basic_entrypoint!(ROUTER);
```

### Key Features

- **Automatic Function Routing**: Generates code to route function calls based on function selectors
- **Argument Handling**: Automatic decoding of function arguments and encoding of results
- **Function ID Support**:
  - Custom function IDs via attributes
  - Solidity signature-based function IDs
- **Fallback Support**: Optional fallback function for unmatched selectors
- **Error Handling**: Built-in handling for unknown function selectors

### Encoding Modes

The router supports two modes of operation:

1. **Solidity Mode**:

```rust
#[router(mode = "solidity")]
```

- Full Solidity ABI compatibility
- Standard 32-byte word size
- Complete Ethereum compatibility

2. **Fluent Mode**:

```rust
#[router(mode = "fluent")]
```

- Compact encoding format
- 4-byte word size
- Optimized for space efficiency

### Function ID Definition and Validation

The router macro supports function identification through custom attributes with built-in validation. The function ID is a 4-byte selector that identifies the function in the contract.

#### Specifying Function IDs

Function IDs can be specified in three ways:

1. **Solidity Signature**:

```rust
#[function_id = "transfer(address,uint256)"]
fn transfer(&self, to: Address, amount: U256) -> bool {
    // implementation
}
```

2. **Direct Hex Value**:

```rust
#[function_id = "0xa9059cbb"]
fn transfer(&self, to: Address, amount: U256) -> bool {
    // implementation
}
```

3. **Raw Byte Array**:

```rust
#[function_id = [169, 5, 156, 187]] // Equivalent to 0xa9059cbb
fn transfer(&self, to: Address, amount: U256) -> bool {
    // implementation
}
```

#### Validation System

The validation system ensures that the specified function ID matches the one calculated from the function signature. This helps prevent routing errors and maintains consistency between your Rust implementation and Solidity interface.

```rust
// With validation (default)
#[function_id("transfer(address,uint256)", validate(true))]

// Without validation
#[function_id("transfer(address,uint256)", validate(false))]
```

##### How Validation Works

1. When validation is enabled (default behavior), the macro:
    - Takes the specified function ID from the attribute
    - Calculates the expected function ID by:
        - Taking the Solidity-style signature of the function
        - Computing the Keccak256 hash of the signature
        - Using the first 4 bytes as the function selector
    - Compares the specified ID with the calculated one
    - Generates a compile-time error if they don't match

For example:

```rust
// This will compile because the ID matches the signature
#[function_id("transfer(address,uint256)")] // ID: 0xa9059cbb
fn transfer(&self, to: Address, amount: U256) -> bool { ... }

// This will also compile because the hex value matches the calculated ID
#[function_id("0xa9059cbb")] // Manually specified ID matching transfer(address,uint256)
fn transfer(&self, to: Address, amount: U256) -> bool { ... }

// This will fail compilation because the ID doesn't match the signature
#[function_id("0x12345678")] // Wrong ID for transfer function
fn transfer(&self, to: Address, amount: U256) -> bool { ... }
```

##### Disabling Validation

You might want to disable validation when:

- Implementing custom routing logic
- Matching an existing contract with different function signatures
- Testing different selector scenarios

```rust
 #[function_id("transfer(string)", validate(false))]
fn greeting(&self, message: String) -> String {
    message
}
```

##### Error Messages

When validation fails:

```rust
 #[function_id("transfer(string)")]
fn greeting(&self, message: String) -> String {
    message
}
```

You'll see compile-time errors like:

```md
Failed to parse method 'greeting': Function ID mismatch for signature 'greeting(string)'. Expected [160, 37, 141, 11], calculated [248, 25, 78, 72]
```

This helps catch potential routing issues early in the development process.

### Method Visibility

The router macro handles methods differently depending on whether you're implementing a trait or writing standalone implementation:

#### Trait Implementation

When implementing a trait, all trait methods are included in routing:

```rust
pub trait RouterAPI {
    fn transfer(&self, params: TransferParams) -> TransferParams;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("transfer((address,uint256,bytes))")]
    fn transfer(&self, params: TransferParams) -> TransferParams {
        params
    }
}
```

#### Direct Implementation

When using direct implementation (without a trait), only public methods are included in routing:

```rust
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ROUTER<SDK> {
    // ✅ Will be included in routing (public)
    #[function_id("transfer((address,uint256,bytes))")]
    pub fn transfer(&self, params: TransferParams) -> TransferParams {
        params
    }

    // ❌ Won't be included (not public)
    #[function_id("private_method()")]
    fn private_method(&self) -> bool {
        true
    }

    // ❌ Won't be included (special method)
    fn deploy(&self) {
        // deployment logic
    }
}
```

### Special Methods

Regardless of implementation type:

- `deploy` method is excluded from routing (used for deployment logic)
- `fallback` method is used as fallback handler if present

### Testing

You can find examples of the `router` macro in the `examples/router-solidity` and `examples/router-fluent` directories.

```rust
#[test]
fn test_contract_works() {
    let message = "Hello World".to_string();
    let greeting_call = GreetingCall::new((message.clone(),));
    let input = greeting_call.encode();

    let sdk = TestingContext::empty().with_input(input);
    let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
    router.deploy();
    router.main();

    let encoded_output = &sdk.take_output();
    let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
    assert_eq!(output.0.0, message);
}
```

### Arguments and Types

The router macro supports automatic type conversion between Solidity and Rust types. Any type that implements both `SolidityType` and `Encoder` traits can be used as an argument.

#### Type Mappings

| Solidity Type | Rust Type        | Notes                                                       |
| ------------- | ---------------- | ----------------------------------------------------------- |
| uint          | u\*              | uint8 to uint256 maps to u8 to U256                         |
| int           | i\*              | int8 to int256 maps to i8 to I256                           |
| bool          | bool             |                                                             |
| address       | Address          |                                                             |
| bytes<M>      | [u8; M]          | Fixed-size byte arrays                                      |
| bytes         | Vec<u8> or Bytes | Dynamic array of bytes                                      |
| string        | String           | UTF-8 encoded                                               |
| tuple         | (T1, T2, ..)     | Rust tuple with equivalent types                            |
| Array<T>      | Vec<T>           | Dynamic array                                               |
| enum          | Enum             | Custom type                                                 |
| mapping       | HashMap<K, V>    | Not directly translatable, use an equivalent data structure |

#### Basic Usage

```rust
#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    // Basic types
    fn transfer(&self, to: Address, amount: U256) -> bool;

    // Strings and bytes
    fn message(&self, text: String, data: Bytes) -> String;

    // Arrays
    fn batch_transfer(&self, recipients: Vec<Address>, amounts: Vec<U256>) -> bool;
}
```

#### Using Structures

!NOT SUPPORTED YET!

<!-- You can use custom structures as param inputs. But, right now we don't support correct function_id derivation for structures. So, you need to manually specify the function_id and set validate to false.

```rust
#[derive(Clone, Debug, Codec)]
pub struct TransferParams {
    pub to: Address,
    pub amount: U256,
    pub data: Vec<u8>,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("transfer((address,uint256,bytes))", validate(false))]
    fn transfer(&self, params: TransferParams) -> bool {
        // Implementation
    }
}
``` -->

## Client Macro

The `client` macro generates type-safe client code for interacting with Solidity-compatible smart contracts. It handles both contract interface and implementation requirements, simplifying encoding and decoding of function calls and supporting no_std environments.

### Key Features

- **Seamless ABI Encoding/Decoding**: Automatically formats data for Solidity ABI.
- **Gas and Value Management**: Provides options for managing gas limits and value transfers.
- **Function ID Support**:
  - Allows both custom function IDs and Solidity signature-based selectors.
- **Deployment Handling**: Optional support for deployment logic.

### Encoding Modes

The client macro supports two encoding modes:

1. **Solidity Mode**: Ensures full Solidity ABI compatibility for Ethereum.
2. **Fluent Mode**: Offers a compact encoding format optimized for space efficiency.

### Example

```rust
#[client(mode = "solidity")]
trait ContractAPI {
    #[function_id("greeting(string)", validate(false))]
    fn greeting(&mut self, message: String) -> String;
}

impl<SDK: SharedAPI> ContractAPIClient<SDK> {
    pub fn greeting_client(
        &mut self,
        contract_address: Address,
        value: U256,
        gas_limit: u64,
        message: String,
    ) -> String {
        self.greeting(contract_address, value, gas_limit, message).0
    }

    pub fn deploy(&self) {
        // deployment logic
    }
}

basic_entrypoint!(ContractAPIClient);
```

### Common Features for Both Macros

- **Encoding Modes**: Both macros support Solidity and Fluent modes for ABI handling.
- **Function ID Options**: Enable custom function IDs with optional validation.

## Storage Macro

The `solidity_storage` macro simplifies the implementation of Solidity's storage patterns in Fluentbase contracts.

### Understanding Solidity Storage

In Solidity, contract storage is organized as a persistent key-value store where each slot is 32 bytes (256 bits):

- For simple types (uint, address, etc.), values are stored sequentially starting from slot 0
- For mappings, the slot contains no value itself, but is used as part of computing storage location for values
- For dynamic arrays, the slot stores the array length, with array data starting at `keccak256(slot)`
- For nested structures, data is stored contiguously starting at a given slot

The `solidity_storage` macro implements these patterns, handling all the complexity of slot management and key calculations.

### Storage Macro Key Features

- **Automatic Slot Management**: Sequential slot assignment starting from 0
- **Type Safety**: Full Solidity type compatibility with Rust type system
- **Key Calculation**: Implements standard Solidity storage layout algorithms
- **Access Methods**: Generates optimized getter and setter methods
- **Support for Complex Types**: Handles mappings, arrays, and custom structures

### Supported Storage Types

1. **Simple Values**:

```rust
solidity_storage! {
    Address Owner; // Slot 0
    U256 Counter; // Slot 1
    Bytes Data;  // Slot 2
}
```

2. **Mappings**:

```rust
solidity_storage! {
    mapping(Address => U256) Balance;                           // Single mapping
    mapping(Address => mapping(Address => U256)) Allowance;     // Nested mapping
}
```

3. **Arrays**:

```rust
solidity_storage! {
    U256[] Array;                  // Single dimension
    Address[][][] NestedArray;     // Multi-dimensional
}
```

4. **Custom Structures**:

```rust
#[derive(Codec, Debug, Default, Clone, PartialEq)]
pub struct MyStruct {
    pub a: U256,
    pub b: U256,
    pub c: Bytes,
    pub d: Bytes,
}

solidity_storage! {
    MyStruct Data;                             // Direct storage
    mapping(Address => MyStruct) StructMap;    // In mapping
}
```

### Type Requirements

All custom types used in storage must implement the `Codec` trait for serialization:

```rust
#[derive(Codec)]  // Required for storage
pub struct MyStruct {
    // fields...
}
```

### Complete Example

> **Note:** You can find a complete example in the `examples/storage` directory.

```rust
use fluentbase_sdk::{codec::Codec, derive::solidity_storage, Address, Bytes, U256};

#[derive(Codec, Debug, Default, Clone, PartialEq)]
pub struct MyStruct {
    pub a: U256,
    pub b: U256,
    pub c: Bytes,
    pub d: Bytes,
}

solidity_storage! {
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
    U256[] Arr;
    Address[][][] NestedArr;
    Address Owner;
    Bytes Data;
    MyStruct SomeStruct;
    mapping(Address => MyStruct) MyStructMap;
}
```

### Testing Storage Layout

```rust
#[test]
pub fn test_storage_mapping_struct() {
    let mut sdk = with_test_input(vec![], None);
    let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

    let my_struct = MyStruct {
        a: U256::from(1),
        b: U256::from(2),
        c: Bytes::from("this is a really long string..."),
        d: Bytes::from("short"),
    };

    MyStructMap::set(&mut sdk, addr, my_struct.clone());
    let result = MyStructMap::get(&sdk, addr);
    assert_eq!(result, my_struct);
}

#[test]
pub fn test_nested_arr() {
    let mut sdk = with_test_input(vec![], None);
    let idx1 = U256::from(0);
    let idx2 = U256::from(0);
    let idx3 = U256::from(0);
    let value = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

    NestedArr::set(&mut sdk, idx1, idx2, idx3, value);
    let result = NestedArr::get(&sdk, idx1, idx2, idx3);
    assert_eq!(result, value);
}
```

### Generated Methods

For each storage variable, the macro generates:

- **Storage Slot**:

```rust
const SLOT: U256 = U256::from_limbs([slot_index, 0, 0, 0]);
```

- **Key Calculation**:

```rust
fn key<SDK: SharedAPI>(sdk: &SDK, ...args) -> U256;
```

- **Getter Method**:

```rust
fn get<SDK: SharedAPI>(sdk: &SDK, ...args) -> T;
```

- **Setter Method**:

```rust
fn set<SDK: SharedAPI>(sdk: &mut SDK, ...args, value: T);
```

### Type Mappings

| Solidity Type | Rust Type     | Storage Layout |
| ------------- | ------------- | -------------- |
| uint256       | U256          | Single slot    |
| address       | Address       | Single slot    |
| bytes         | Bytes         | Single slot    |
| mapping       | -             | Hashed slots   |
| array         | -             | Sequential     |
| struct        | Custom type\* | Multiple slots |

\* Custom types must implement `Codec` trait

### Notes

1. **Custom Types**: All user-defined types must implement `Codec` trait
2. **Slot Assignment**: Automated and sequential for all storage variables
3. **Key Calculation**: Follows Solidity's standard algorithm for compatibility
4. **Gas Optimization**: Generated code is optimized for minimal gas usage

The macro handles all the low-level storage management, allowing you to focus on your contract's business logic while maintaining full compatibility with Solidity's storage layout.
