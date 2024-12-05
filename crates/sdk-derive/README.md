# Sdk derive macro

## Router macro

Generates a router for Fluentbase contract functions with configurable encoding modes.

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
5. Handle potential errors
6. Encode and return results

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

Example of using the router macro:

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

### Constraints

- References and mutable types are not allowed for function parameters
- All function parameters and return types must implement `fluentbase_sdk::codec::Encoder` trait ([Encoder](fluentbase_sdk::codec::Encoder))
- Custom error handling is available through fallback function

### Arguments

- `mode` - Encoding mode: "solidity" or "fluent"

### Usage Examples

#### Regular Implementation

Only public functions will be dispatched:

```rust
#[router(mode = "solidity")]
impl<SDK: SharedAPI> ROUTER<SDK> {
    pub fn greeting(&mut self, data: FixedBytes<32>) {
        self.sdk.write(data.as_slice());
    }
}
```

#### Trait Implementation

All functions will be dispatched:

```rust
trait RouterAPI {
    fn greeting(&self, msg: String) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    fn greeting(&self, msg: String) -> String {
        msg
    }
}
```

### Attributes

Function-level attributes:

- `#[function_id("fnName(types)")]` - Specify custom function selector (optional). See [function_id](../README.md#function_id-attribute) for details

### Error Handling

Compile-time errors occur when:

- Invalid encoding mode is specified
- Function signatures contain incompatible types
- Incompatible function_id is specified

### Generated Output

The macro generates:

- Function selector matching logic
- Input parameter decoding
- Result encoding
- Error handling for unknown selectors

For complete implementation details, see [documentation](../README.md#router-macro).
