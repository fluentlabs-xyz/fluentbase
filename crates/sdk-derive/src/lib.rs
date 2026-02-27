//! Procedural macros for routers, clients, events, and storage layouts in Fluentbase contracts.
use fluentbase_sdk_derive_core::{
    client, event, router, storage::process_storage_layout, storage_legacy,
};
use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use quote::{quote, ToTokens};
use syn::parse_macro_input;

mod utils;

/// Function ID attribute for overriding function selectors in smart contracts.
///
/// Specifies a custom 4-byte selector that identifies function calls in the ABI.
///
/// # Formats
/// - Solidity signature: `#[function_id("transfer(address,uint256)")]`
/// - Hex string: `#[function_id("0xa9059cbb")]`
/// - Byte array: `#[function_id([169, 5, 156, 187])]`
///
/// # Validation
/// Optional validation ensures the selector matches the function signature:
///
/// ```rust,ignore
/// // Verify hex selector matches the function signature
/// #[function_id("0xa9059cbb", validate(true))]
/// fn transfer(&self, to: Address, amount: U256) -> bool { ... }
///
/// // Verify function implementation matches the signature
/// #[function_id("transfer(address,uint256)", validate(true))]
/// fn transfer(&self, to: Address, amount: U256) -> bool { ... }
/// ```
///
/// Validation is useful when:
/// - Ensuring type conversions produce the expected selector
/// - Maintaining compatibility with existing contracts
/// - During code refactoring to catch selector changes
///
/// By default, validation is disabled and selectors are used as-is.
///
/// # Example
/// ```rust,ignore
/// #[function_id("greeting(string)")]
/// fn greeting(&self, message: String) -> String {
///     message
/// }
/// ```
#[proc_macro_attribute]
pub fn function_id(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
/// Router macro for Fluentbase smart contracts.
///
/// Automatically creates a method dispatch system that routes incoming function calls
/// based on their 4-byte selectors, handling parameter decoding and result encoding.
///
/// # Usage
///
/// ## Trait Implementation
///
/// ```rust,ignore
/// #[router(mode = "solidity")]
/// impl<SDK: SharedAPI> ContractTrait for Contract<SDK> {
///     #[function_id("greeting(string)")]
///     fn greeting(&self, message: String) -> String {
///         message
///     }
/// }
/// ```
///
/// ## Direct Implementation
///
/// ```rust,ignore
/// #[router(mode = "solidity")]
/// impl<SDK: SharedAPI> Contract<SDK> {
///     // Only public methods are included in routing
///     pub fn store(&mut self, value: U256) {
///         // Implementation
///     }
///
///     // Private methods are excluded from routing
///     fn internal_helper(&self) {
///         // Implementation
///     }
/// }
/// ```
///
/// # Special Methods
///
/// - **deploy**: Always excluded from routing, used for initialization ```rust,ignore fn
///   deploy(&self) { // Deployment logic, called only once } ```
///
/// - **fallback**: Handles unmatched selectors ```rust,ignore fn fallback(&self) { // Called for
///   unknown function selectors } ```
///
/// # Attributes
///
/// - **mode**: Encoding mode
///   - `"solidity"`: Full EVM compatibility
///   - `"fluent"`: Optimized for WASM (smaller payloads)
///
/// # Function Selectors
///
/// Methods are automatically assigned selectors based on their signatures,
/// or can use custom selectors with `#[function_id(...)]`.
///
/// Selector collisions are detected at compile time.
#[proc_macro_attribute]
#[proc_macro_error]
pub fn router(attr: TokenStream, input: TokenStream) -> TokenStream {
    match router::process_router(attr.into(), input.into()) {
        Ok(router) => router.to_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}
/// Client macro for type-safe interaction with Fluentbase contracts.
///
/// Generates a client struct and methods from a trait definition, handling contract
/// calls, parameter encoding, and result decoding automatically.
///
/// # Usage
///
/// ```rust,ignore
/// #[client(mode = "solidity")]
/// trait TokenInterface {
///     #[function_id("balanceOf(address)")]
///     fn balance_of(&self, owner: Address) -> U256;
///
///     #[function_id("transfer(address,uint256)")]
///     fn transfer(&mut self, to: Address, amount: U256) -> bool;
/// }
///
/// // Using the generated client
/// let mut client = TokenInterfaceClient::new(sdk);
///
/// // Call contract methods with standard parameters
/// let balance = client.balance_of(
///     token_address,    // contract address
///     U256::zero(),     // value to send (none)
///     50000,            // gas limit
///     my_address        // method-specific parameters
/// );
/// ```
///
/// # Generated Code
///
/// For a trait named `TokenInterface`, generates:
///
/// - `TokenInterfaceClient<SDK>` struct with a `new(sdk)` constructor
/// - Method implementations that append common parameters: ```rust,ignore fn method_name( &mut
///   self, contract_address: Address,  // Target contract value: U256,                // Native
///   tokens to send gas_limit: u64,             // Maximum gas ...original_parameters      // From
///   trait definition ) -> original_return_type ```
///
/// # Features
///
/// - **Automatic encoding/decoding** of parameters and return values
/// - **Runtime safety checks** for insufficient funds or gas
/// - **Compatible with router** when using the same encoding mode
/// - **Preserves method signatures** from the trait definition
///
/// # Attributes
///
/// - **mode**: Encoding mode
///   - `"solidity"`: Full EVM compatibility (default)
///   - `"fluent"`: Optimized encoding for WASM
#[proc_macro_attribute]
#[proc_macro_error]
pub fn client(attr: TokenStream, input: TokenStream) -> TokenStream {
    let attr_ts = proc_macro2::TokenStream::from(attr);
    let input_items = parse_macro_input!(input as syn::ItemTrait);

    match client::process_client(attr_ts, input_items.to_token_stream()) {
        Ok(client) => client.to_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Implements Solidity-compatible storage in Fluentbase contracts.
///
/// Provides an efficient, type-safe storage system following Solidity's storage layout,
/// with automatic slot assignment and optimized access methods.
///
/// # Usage
///
/// ```rust,ignore
/// solidity_storage! {
///     // Simple values
///     Address Owner;                         // Slot 0
///     bool Paused;                           // Slot 1
///
///     // Mappings
///     mapping(Address => U256) Balance;      // Slot 2
///     mapping(Address => mapping(Address => U256)) Allowance; // Slot 3
///
///     // Arrays
///     U256[] Values;                         // Slot 4
/// }
///
/// // Using the generated storage
/// impl<SDK: SharedAPI> Contract<SDK> {
///     fn transfer(&mut self, to: Address, amount: U256) -> bool {
///         let sender = self.sdk.caller();
///         let sender_balance = Balance::get(&self.sdk, sender);
///
///         if sender_balance >= amount {
///             Balance::set(&mut self.sdk, sender, sender_balance - amount);
///             Balance::set(&mut self.sdk, to, Balance::get(&self.sdk, to) + amount);
///             true
///         } else {
///             false
///         }
///     }
/// }
/// ```
///
/// # Features
///
/// - **Key calculation**: Automatically implements correct hashing for mappings and arrays
/// - **Optimization**: Uses direct storage access for types ≤ 32 bytes
/// - **Type safety**: Generates properly typed getters and setters
/// - **Full type support**: Works with all Solidity types including nested structures
///
/// # Generated API
///
/// For each variable `Name`, generates:
///
/// - A struct with slot constants: `struct Name { ... }`
/// - Getter: `Name::get(sdk, ...arguments)` → returns the stored value
/// - Setter: `Name::set(sdk, ...arguments, new_value)` → updates the stored value
///
/// The arguments depend on the storage type:
/// - Simple values: no arguments
/// - Mappings: one argument per key level
/// - Arrays: indices for each dimension
#[proc_macro]
#[proc_macro_error]
pub fn solidity_storage(input: TokenStream) -> TokenStream {
    let storage = parse_macro_input!(input as storage_legacy::Storage);
    storage.to_token_stream().into()
}

/// Generates Rust traits from Solidity interfaces and contracts.
///
/// Automatically converts Solidity definitions to Rust, preserving function
/// signatures and type mappings, for seamless integration with Rust contracts.
///
/// # Usage
///
/// From a file:
/// ```rust,ignore
/// // Import from a .sol file
/// derive_solidity_trait!("abi/IERC20.sol");
/// ```
///
/// Or inline:
/// ```rust,ignore
/// // Define directly in Rust
/// derive_solidity_trait!(
///     interface IERC20 {
///         function balanceOf(address account) external view returns (uint256);
///         function transfer(address to, uint256 amount) external returns (bool);
///     }
/// );
///
/// // Use with router
/// #[router(mode = "solidity")]
/// impl<SDK: SharedAPI> IERC20 for MyToken<SDK> {
///     fn balance_of(&self, account: Address) -> U256 {
///         // Implementation
///     }
///
///     fn transfer(&mut self, to: Address, amount: U256) -> bool {
///         // Implementation
///     }
/// }
/// ```
///
/// # Features
///
/// - **Automatic type conversion**: Solidity → Rust types
/// - **Name conversion**: camelCase → snake_case for methods
/// - **Method receivers**: `&self` for view/pure, `&mut self` for others
/// - **Struct support**: Generates Rust structs for Solidity structs
/// - **Works with router**: Use traits in `#[router]` implementations
#[proc_macro]
#[proc_macro_error]
pub fn derive_solidity_trait(input: TokenStream) -> TokenStream {
    let parsed = syn::parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    fluentbase_sdk_derive_core::sol_input::to_rust_trait(parsed)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generates ready-to-use client code from Solidity definitions.
///
/// Creates a complete client implementation that can interact with
/// deployed contracts, combining `derive_solidity_trait` and `client`
/// functionality in one step.
///
/// # Usage
///
/// ```rust,ignore
/// // Generate client from Solidity interface
/// derive_solidity_client!(
///     interface IERC20 {
///         function balanceOf(address account) external view returns (uint256);
///         function transfer(address to, uint256 amount) external returns (bool);
///     }
/// );
///
/// // Use the generated client
/// fn example<SDK: SharedAPI>(sdk: SDK) {
///     let mut client = IERC20Client::new(sdk);
///
///     // Check balance
///     let balance = client.balance_of(
///         token_address,    // contract to call
///         U256::zero(),     // no value to send
///         50000,            // gas limit
///         my_address        // function parameter
///     );
///
///     // Transfer tokens
///     if balance > amount {
///         let success = client.transfer(
///             token_address, U256::zero(), 50000,
///             recipient, amount
///         );
///     }
/// }
/// ```
///
/// # Features
///
/// - **One-step generation**: Creates both trait and client implementation
/// - **Type-safe methods**: Enforces correct parameter and return types
/// - **Standard parameters**: All methods include contract address, value, and gas limit
/// - **Full Solidity support**: Works with structs, arrays, and complex types
/// - **File loading**: Can import from `.sol` files with
///   `derive_solidity_client!("path/to/file.sol")`
#[proc_macro]
#[proc_macro_error]
pub fn derive_solidity_client(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    fluentbase_sdk_derive_core::sol_input::to_sol_client(parsed)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Calculates a Keccak-256 function selector from a signature.
///
/// Returns the first 4 bytes of the Keccak-256 hash as a u32 value.
///
/// # Example
///
/// ```rust,ignore
/// let selector = derive_keccak256_id!("transfer(address,uint256)");
/// // Returns 0xa9059cbb
/// ```
#[proc_macro]
pub fn derive_keccak256_id(token: TokenStream) -> TokenStream {
    let signature = token.to_string();
    let method_id = utils::calculate_keccak256_id(&signature);
    TokenStream::from(quote! {
        #method_id
    })
}

/// Calculates a full Keccak-256 hash from a signature.
///
/// Returns the complete 32-byte Keccak-256 hash as a byte array.
///
/// # Example
///
/// ```rust,ignore
/// let hash = derive_keccak256!("Transfer(address,address,uint256)");
/// // Returns the complete hash as [u8; 32]
/// ```
#[proc_macro]
pub fn derive_keccak256(token: TokenStream) -> TokenStream {
    let signature = token.to_string();
    let method_id = utils::calculate_keccak256(&signature);
    TokenStream::from(quote! {
        [#(#method_id,)*]
    })
}

#[proc_macro]
pub fn derive_evm_error(token: TokenStream) -> TokenStream {
    let signature = token.to_string();
    let method_id = utils::calculate_keccak256_id(&signature);
    TokenStream::from(quote! {
        #method_id
    })
}

fn derive_storage_layout(input: TokenStream) -> TokenStream {
    let input = syn::parse(input).unwrap();
    match process_storage_layout(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derives storage layout for nested structures.
///
/// Use for composite storage types within contracts.
/// For main contracts, use `#[derive(Contract)]` instead.
///
/// # Field Attributes
///
/// ## `#[slot(expr)]`
///
/// Places field at explicit storage slot. The expression must evaluate to `U256`.
///
/// ```rust,ignore
/// use fluentbase_sdk::derive::{Storage, eip1967_slot};
///
/// pub const MY_SLOT: U256 = eip1967_slot!("eip1967.proxy.implementation");
///
/// #[derive(Storage)]
/// struct Config {
///     #[slot(MY_SLOT)]
///     implementation: StorageAddress,
///
///     // Auto-layout continues normally
///     owner: StorageAddress,
/// }
/// ```
///
/// Fields with explicit slots:
/// - Do not affect auto-layout counter
/// - Are excluded from `SLOTS` constant
/// - Must use `U256` type (compile error otherwise)
///
/// # Example
/// ```rust,ignore
/// #[derive(Storage)]
/// struct Config {
///     owner: StoragePrimitive<Address>,
///     version: StoragePrimitive<u32>,
/// }
/// ```
#[proc_macro_derive(Storage, attributes(slot))]
pub fn derive_storage(input: TokenStream) -> TokenStream {
    derive_storage_layout(input)
}

/// Derives contract implementation with storage support.
///
/// Generates initialization and storage accessor methods for contract structs.
/// For nested storage structures, use `#[derive(Storage)]` instead.
///
/// # Field Attributes
///
/// ## `#[slot(expr)]`
///
/// Places field at explicit storage slot for EIP-1967 proxy patterns
/// or ERC-7201 namespaced storage.
///
/// ```rust,ignore
/// use fluentbase_sdk::derive::{Contract, eip1967_slot, erc7201_slot};
///
/// pub mod slots {
///     use fluentbase_sdk::derive::eip1967_slot;
///     pub const IMPLEMENTATION: U256 = eip1967_slot!("eip1967.proxy.implementation");
/// }
///
/// #[derive(Contract)]
/// pub struct Proxy<SDK> {
///     sdk: SDK,
///
///     #[slot(slots::IMPLEMENTATION)]
///     implementation: StorageAddress,
///
///     // Auto-layout fields
///     owner: StorageAddress,
///     counter: StorageU256,
/// }
/// ```
///
/// # Helper Macros
///
/// - [`eip1967_slot!`] - Computes EIP-1967 slot: `keccak256(id) - 1`
/// - [`erc7201_slot!`] - Computes ERC-7201 slot: `keccak256(keccak256(id) - 1) & ~0xff`
///
/// # Example
/// ```rust,ignore
/// #[derive(Contract)]
/// pub struct MyToken<SDK> {
///     sdk: SDK,
///     total_supply: StorageU256,
///     balances: StorageMap<Address, U256>,
/// }
/// ```
#[proc_macro_derive(Contract, attributes(slot))]
pub fn derive_contract(input: TokenStream) -> TokenStream {
    derive_storage_layout(input)
}

/// Computes EIP-1967 storage slot at compile time.
/// Formula: keccak256(id) - 1
///
/// See: https://eips.ethereum.org/EIPS/eip-1967
///
/// # Example
/// ```rust,ignore
/// use fluentbase_sdk::derive::eip1967_slot;
///
/// const IMPL_SLOT: U256 = eip1967_slot!("eip1967.proxy.implementation");
/// ```
#[proc_macro]
pub fn eip1967_slot(input: TokenStream) -> TokenStream {
    let lit = syn::parse_macro_input!(input as syn::LitStr);
    let id = lit.value();

    let hash = utils::calculate_keccak256(&id);

    // keccak256(id) - 1
    let mut bytes = hash;
    let mut borrow = true;
    for i in (0..32).rev() {
        if borrow {
            if bytes[i] == 0 {
                bytes[i] = 0xff;
            } else {
                bytes[i] -= 1;
                borrow = false;
            }
        }
    }

    TokenStream::from(quote! {
        fluentbase_sdk::U256::from_be_bytes([#(#bytes),*])
    })
}

/// Computes ERC-7201 namespaced storage slot at compile time.
/// Formula: keccak256(abi.encode(uint256(keccak256(id)) - 1)) & ~0xff
///
/// See: https://eips.ethereum.org/EIPS/eip-7201
///
/// # Example
/// ```rust,ignore
/// use fluentbase_sdk::derive::erc7201_slot;
///
/// const STORAGE_SLOT: U256 = erc7201_slot!("example.main");
/// ```
#[proc_macro]
pub fn erc7201_slot(input: TokenStream) -> TokenStream {
    let lit = syn::parse_macro_input!(input as syn::LitStr);
    let id = lit.value();

    // Step 1: keccak256(id)
    let inner = utils::calculate_keccak256(&id);

    // Step 2: subtract 1
    let mut shifted = inner;
    let mut borrow = true;
    for i in (0..32).rev() {
        if borrow {
            if shifted[i] == 0 {
                shifted[i] = 0xff;
            } else {
                shifted[i] -= 1;
                borrow = false;
            }
        }
    }

    // Step 3: keccak256(shifted)
    let mut outer = utils::calculate_keccak256_raw::<32>(&shifted);

    // Step 4: & ~0xff (clear last byte)
    outer[31] = 0;

    TokenStream::from(quote! {
        fluentbase_sdk::U256::from_be_bytes([#(#outer),*])
    })
}

/// Defines contract initialization logic.
///
/// Generates a `deploy()` entry point that handles parameter decoding
/// during contract deployment. Use when implementing traits or when
/// you need initialization separate from runtime methods.
///
/// # Example
/// ```rust,ignore
/// #[constructor(mode = "solidity")]
/// impl<SDK: SharedAPI> MyContract<SDK> {
///     pub fn constructor(&mut self, owner: Address, supply: U256) {
///         // Initialization logic
///     }
/// }
/// ```
///
/// # Attributes
/// - `mode`: `"solidity"` (EVM) or `"fluent"` (optimized)
#[proc_macro_attribute]
#[proc_macro_error]
pub fn constructor(attr: TokenStream, input: TokenStream) -> TokenStream {
    match fluentbase_sdk_derive_core::constructor::process_constructor(attr.into(), input.into()) {
        Ok(constructor) => constructor.to_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derives Solidity-compatible event emission for structs.
///
/// # Example
/// ```rust,ignore
/// #[derive(Event)]
/// struct Transfer {
///     #[indexed]
///     from: Address,
///     #[indexed]
///     to: Address,
///     value: U256,
/// }
///
/// Transfer { from, to, value }.emit(&mut sdk);
/// ```
#[proc_macro_derive(Event, attributes(indexed, anonymous))]
#[proc_macro_error]
pub fn derive_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    match event::process_event(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
