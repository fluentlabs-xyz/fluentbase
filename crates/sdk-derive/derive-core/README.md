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

Router supports 2 modes - `solidity` and `fluent`. The main logic the same for both modes, but the way how we decode arguments is different.

We are using [fluent codec](https://github.com/fluentlabs-xyz/codec2). This codec also supports 2 modes:

- `solidity` - fully compatitable with SolidityABI
- `fluent` - compact codec, that very similar to SolidityABI, but it has some differences. For example, the word size is 4 bytes instead of 32. See [fluent codec](https://github.com/fluentlabs-xyz/codec2) for more details.

`examples/router-solidity` - shows how to use `router` macro in `solidity` mode.
`examples/router-wasm` - shows how to use `router` macro in `fluent` mode.

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

## Macro tips

| To | From | Method |
|---|---|---|
| string of code | literal code | `std::stringify!{}` |
| string of code | syntax tree | `!quote(#syntax_tree).to_string()` |
| string of code | TokenStream | `.to_string()` |
| string of syntax | literal code | `format!("{:?}",parse2::<SynType>(quote! {...}).expect(...))` |
| string of syntax | syntax tree | `format!("{:?}",…), format!("{:#?}",…)` |
| string of tokens | literal code | `format!("{:?}",quote! {...})` |
| string of tokens | TokenStream | `format!("{:?}",…), format!("{:#?}",…)` |
| syn::Error | TokenStream | `.to_compile_error()` [см. Правило #7] |
| syntax tree | literal code | `parse_quote!(…)` |
| syntax tree | proc_macro::TokenStream | `parse_macro_input!(…), parse` |
| syntax tree | string of code | `parse_str(…)` |
| syntax tree | TokenStream | `parse2::<SynType>(…), etc` |
| TokenStream | literal code | `quote!(…)` |
| TokenStream | string of code | `parse_str(…)` |
| TokenStream | syntrax tree | `quote!(#syntax_tree)` или `.to_token_stream(),` |
| TokenStream | TokenStream | `.into(), ::from(...)` [см. Правило #1] |
