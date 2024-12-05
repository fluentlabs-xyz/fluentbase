# Router Macro

Generates a router for Fluentbase contract functions with configurable encoding modes.

## How Contract Execution Works

In Fluentbase, contracts have two system functions:

- `deploy`: executed once during contract deployment
- `main`: serves as the primary entry point for all contract interactions

When a contract is called, the `main` function receives input data where:

- The first 4 bytes represent a function selector
- The remaining bytes contain the encoded function parameters

## Why Router is Needed

Without the router macro, developers would need to write boilerplate code to:

1. Read and validate input data length
2. Extract the function selector (first 4 bytes)
3. Match the selector with the appropriate function
4. Decode function parameters
5. Execute the matched function with decoded parameters
6. Handle potential errors
7. Encode and return results

Here's an example of what this would look like in raw implementation:

```rust
impl<SDK: SharedAPI> ROUTER<SDK> {
    pub fn main(&mut self) {
        let input_length = self.sdk.input_size();
        if input_length < 4 {
            panic!("insufficient input length for method selector");
        }
        let mut call_data = fluentbase_sdk::alloc_slice(input_length as usize);
        self.sdk.read(&mut call_data, 0);
        let (selector, params) = call_data.split_at(4);
        match [selector[0], selector[1], selector[2], selector[3]] {
            GreetingCall::SELECTOR => {
                let (data) = match GreetingCall::decode(&params) {
                    Ok(decoded) => (decoded.0.0),
                    Err(_) => panic!("Failed to decode input parameters"),
                };
                self.greeting(data);
                self.sdk.write(&[0u8; 0]);
            }
            _ => panic!("unsupported method selector"),
        }
    }
}
```

## How Router Helps

The router macro automates this entire process by generating all the necessary boilerplate code. With the macro, developers can focus on implementing contract logic instead of handling low-level routing details.

## Implementation Types

The router supports two ways of implementing contract functions:

### Struct Implementation

When implementing functions directly for the struct, the router only includes public functions in the dispatch mechanism. This ensures that only explicitly exposed functions can be called externally:

```rust
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ROUTER<SDK> {
    pub fn external_method(&mut self, data: String) -> String {
        // This function will be routed
        data
    }

    fn internal_helper(&self) -> bool {
        // This function will NOT be routed
        true
    }
}
```

### Trait Implementation

When implementing a trait, all functions from the trait become part of the dispatch mechanism:

```rust
trait RouterAPI {
    fn greeting(&self, message: String) -> String;
    fn farewell(&self) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    // All trait functions will be routed
    fn greeting(&self, message: String) -> String {
        message
    }

    fn farewell(&self) -> String {
        "goodbye".to_string()
    }
}
```

## Encoding Modes

Each mode uses 4-byte function selectors for routing and implements parameter encoding/decoding according to its rules:

### Solidity Mode

```rust
#[router(mode = "solidity")]
```

- Big-endian byte order
- 32-byte alignment
- Compatible with Ethereum ABI specification

### Fluent Mode

```rust
#[router(mode = "fluent")]
```

- Little-endian byte order
- 4-byte alignment
- Optimized for compact data representation

## Function Selectors

By default, each function's selector is calculated by taking the first 4 bytes of the keccak256 hash of its signature (excluding the receiver). Function names are converted to camelCase before hashing. For example, a function `pub fn transfer_all(&self, to: Address, amount: U256)` will be hashed as `transferAll(address,uint256)`.

You can override the default selector using the `function_id` attribute in three formats:

```rust
// Using Solidity-style signature (will be hashed)
#[function_id("transferAll(address,uint256)")]
fn transfer_all(&self, to: Address, amount: U256)

// Using direct hex representation
#[function_id("0xa9059cbb")]
fn transfer_all(&self, to: Address, amount: U256)

// Using raw bytes
#[function_id([169, 5, 156, 187])]
fn transfer_all(&self, to: Address, amount: U256)
```

By default, when using the signature format, the router validates that the provided signature matches the function's actual selector. You can disable this validation with `validate(false)`:

```rust
#[function_id("transferAll(address,uint256)", validate(false))]
fn transfer_all(&self, to: Address, amount: U256)
```

## Generated Artifacts

The macro automatically generates `{ContractName}.abi.json` and `I{ContractName}.sol` in `OUT_DIR`. To save artifacts in a local directory, you can either:

1. Use a `build.rs` script:

```rust
use std::{env, fs};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let artifacts_dir = env::var("FLUENTBASE_CONTRACT_ARTIFACTS_DIR")
        .unwrap_or_else(|_| format!("{}/artifacts", manifest_dir));

    fs::create_dir_all(&artifacts_dir).expect("Failed to create artifacts directory");
    println!("cargo:rustc-env=FLUENTBASE_CONTRACT_ARTIFACTS_DIR={}", artifacts_dir);
}
```

2. Manually set the `FLUENTBASE_CONTRACT_ARTIFACTS_DIR` environment variable

## Special Functions

### Fallback Handler

You can implement a fallback function that will be called only when the function selector doesn't match any known function:

```rust
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ROUTER<SDK> {
    pub fn fallback(&mut self) {
        // Handle unknown selectors
    }
}
```

If the selector is known but other errors occur (e.g., decoding errors), the router will panic instead of calling fallback.

### Deploy Function

The `deploy` function is always excluded from routing as it's used only once during contract deployment.

## Type Mapping

The router maps Rust types to Solidity ABI types and generates all necessary encoding/decoding logic.

### Basic Types

```rust
// Integers
u8, u16, u32, u64, u128, U256 -> uint8, uint16, uint32, uint64, uint128, uint256
i8, i16, i32, i64, i128, I256 -> int8, int16, int32, int64, int128, int256

// Other primitives
bool -> bool
Address -> address
String, str -> string
Bytes -> bytes
FixedBytes<N> -> bytesN  // where N is 1 to 32
```

### Composite Types

```rust
// Dynamic and fixed arrays
Vec<T> -> T[]
[T; N] -> T[N]

// Tuples
(T1, T2) -> tuple(T1, T2)

// Custom structs
struct Point { x: U256, y: U256 }
// Encoded as: tuple(uint256, uint256)
```

### Requirements

- All types must implement `fluentbase_sdk::codec::Encoder`
- References (`&T`, `&mut T`) are not allowed in function parameters
- The router automatically generates necessary encoding/decoding for parameters and return values

Давайте добавим новый раздел после Type Mapping:

## Generated Codecs

For each function, the router generates encoding/decoding types for parameters and return values. For example, given this function:

```rust
fn greeting(&mut self, a: FixedBytes<32>, b: U256) -> U256
```

### Parameter Handling

```rust
// Type alias for function parameters
pub type GreetingCallArgs = (FixedBytes<32>, U256);

// Parameter codec with function selector
pub struct GreetingCall(GreetingCallArgs) {
    pub const SELECTOR: [u8; 4] = [...];
    pub const SIGNATURE: &'static str = "greeting(bytes32,uint256)";
}

// Generate input data for contract call
let input = GreetingCall::new((
    FixedBytes::<32>::repeat_byte(0xAA),
    U256::from(1)
)).encode();
```

### Return Value Handling

```rust
// Type alias for return values
pub type GreetingReturnArgs = (U256,);

// Return value codec
pub struct GreetingReturn(GreetingReturnArgs) {
    pub fn encode(&self) -> Bytes;
    pub fn decode(buf: &impl Buf) -> Result<Self, CodecError>;
}
```

The router handles all ABI encoding details including selector prefixing and dynamic type offsets.
