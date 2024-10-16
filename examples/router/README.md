# Solidity router example

This example shows how to use solidity_router macro.

## Solidity Router  macro

In Fluentbase, interfacing with a contract is streamlined through the use of the `main` function, which acts as the primary entry point for all contract interactions. To communicate with a contract, transactions must include input data, comprised of a function selector and its corresponding arguments. The function selector is derived as the first 4 bytes of the keccak256 hash of the function's signature, while arguments are formatted according to the Solidity ABI standard.

Upon receiving a transaction, the `main` function is triggered. It carries the responsibilities of decoding the input data, routing the call to the appropriate function within the contract, and encoding the result before sending it back.

To alleviate the repetitive nature of writing such entry point logic manually, Fluentbase provides a macro called `solidity_router`. This macro automates the generation of boilerplate code needed for the dispatch mechanism. By supplying a list of functions, the `solidity_router` macro not only creates the necessary routing logic but also handles argument decoding, function invocation, and result encoding seamlessly. For instance, the following implementation will result in automatic generation of the essential code structure for function calls. This setup significantly reduces manual overhead and potential for error, allowing developers to focus more on the core logic of their smart contracts.

## Key Features

- **Automatic function routing**: The `solidity_router` macro generates the necessary code to route function calls based on the function selector.
- **Argument decoding**: The macro automatically decodes function arguments according to the Solidity ABI standard.
- **Result encoding**: The macro encodes the result of a function call before sending it back to the caller.
- **Function ID calculation**: The function ID can be passed as an attribute to the function, allowing for custom function IDs.
- **Solidity signature support**: The function ID can also be passed as a Solidity signature, enabling the use of function signatures for routing.
- **Fallback function support**: If original trait has a function with the name `fallback`, it will be used as a fallback function.
- **Error handling**: The macro includes basic error handling for unknown function selectors.

## Usage

To use the `solidity_router` macro, you need to define a trait with the desired functions and then implement it for a struct representing the contract. The macro will generate the necessary routing logic based on the functions defined in the trait. Also for each function

For example, this code:

```rust
trait ContractA {
    fn foo(&self, a: u32, b: u32) -> u32;
    fn bar(&self, a: u32, b: u32) -> u32;
}

#[solidity_router]
impl ContractA for Contract {
    fn foo(&self, a: u32, b: u32) -> u32 {
        a + b
    }

    fn bar(&self, a: u32, b: u32) -> u32 {
        a * b
    }
}
```

Will expand to the following code:

```rust
trait ContractA {
    fn foo(&self, a: u32, b: u32) -> u32;
}
impl ContractA for Contract {
    fn foo(&self, a: u32, b: u32) -> u32 {
        a + b
    }

}

type FooCall = (u32,u32,);
type FooReturn = u32;


impl FooCall {
    const SELECTOR: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
    const SIGNATURE: &str = "foo(uint32,uint32)";
}

impl Contract {
    fn main() {
        let input_size = self.sdk.input_size();
        if input_size < 4 {
            panic!("input too short");
        }
        let mut input_vec = fluentbase_sdk::alloc_slice(input_size as usize);
        self.sdk.read(&mut input_vec, 0);
        let mut full_input = fluentbase_sdk::alloc_slice(input_size as usize);
        self.sdk.read(&mut full_input, 0);
        let (selector, data_input) = full_input.split_at(4);
        let selector: [u8; 4] = selector.try_into().expect("Selector should be 4 bytes");

        match selector {
            // function ID
            fooCall::SELECTOR => {
                // decode args
                 let decoded_result = SolidityABI::<FooCall>::decode(&data_input, 0);

                let message = match decoded_result {
                    Ok(decoded) => decoded.0,
                    Err(_) => {
                        panic!("failed to decode input");
                    }
                };
                // call function
                let result = self.foo(message);
                // encode function result
                let buf = BytesMut::new();
                SolidityABI::<FooReturn>::encode(&result, buf);
                let result = buf.freeze();
                // write result to output
                sdk.write(&result);
            },
            _ => panic!("Unknown function selector"),
        }
    }
}
```

### FAQ

#### How function_id is calculated?

The function ID can be passed as an attribute `function_id`:

```rust
#[function_id = "0x12345678"]
fn foo(&self, a: u32, b: u32) -> u32 {
    a + b
}
```

You can pass function ID as a solidity signature as well. Keep in mind that the function signature should be in the short format `function_name(type1,type2,...)`. For example:

```rust
#[function_id = "foo(uint32,uint32)"]
fn foo(&self, a: u32, b: u32) -> u32 {
    a + b
}
```

> **NOTE:** Function ID used only for routing. So the amount of the arguments in the signature and actual method may differ. To figure out of how we decode method arguments see section below.

#### How arguments are decoded?

For most cases it's pretty simple. We can use as arguments any type that implements `SolidityType` + `Encoder` traits. For example, if we have a function with the following signature:

```rust
fn foo(&self, a: Address, b: Bytes) -> U256 {
    // some code
}
```

The function ID is calculated as `foo(address,bytes)`. So to decode arguments we need to make a simple mapping:

| Solidity Type | Rust Type | Notes |
|---------------|---------------|--------------------------------|
| uint | u*| uint8 to uint256 maps to u8 to U256 |
| int | i* | int8 to int256 maps to i8 to I256 |
| bool | bool | |
| address | Address |  |
| bytes<M> | [u8; M] | Fixed-size byte arrays |
| bytes | Vec<u8> or Bytes | Dynamic array of bytes |
| string | String | UTF-8 encoded |
| tuple | (T1, T2, ..)| Rust tuple with equivalent types |
| Array<T> | Vec<T> | Dynamic array |
| enum | Enum | Custom type |
| mapping | HashMap<K, V> | Not directly translatable, use an equivalent data structure |

you can access fooCall::SELECTOR and fooCall:Signature to ensure that it looks like what you expect.

Sometimes it's can be a bit tricky. For example, if we have a function with the following signature:

TODO:

```rust
type fooCall = (u32,u32,);
let decoded_result = SolidityABI::<fooCall>::decode(&data_input, 0);
```

#### How to map Solidity Type to Rust or what should I use as arguments for contract methods?

Short answer that you can use any type that implements `SolidityType` + `Encoder` traits.

The mapping is pretty straightforward:
