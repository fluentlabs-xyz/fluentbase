use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use proc_macro_error::{abort, abort_call_site};
use quote::{quote, ToTokens};
use std::num::NonZeroU16;
use syn::{
    ext::IdentExt, // Import IdentExt for peek_any
    parse::{discouraged::Speculative, Parse, ParseStream},
    punctuated::Punctuated,
    token::Semi,
    Error,
    Result as SynResult,
    Token,
};
use syn_solidity::{Type, TypeArray, TypeMapping};

/// Storage parameter with type information.
/// Used to define parameters for storage getter and setter methods.
#[derive(Clone, Debug)]
pub struct StorageParam {
    /// Parameter name
    name: Ident,
    /// Parameter type
    ty: Ident,
}

impl StorageParam {
    /// Creates a new storage parameter with default span
    pub fn new(name: impl Into<String>, ty: impl Into<String>) -> Self {
        Self::with_span(name, ty, Span::call_site())
    }

    /// Creates a new storage parameter with a specific span
    pub fn with_span(name: impl Into<String>, ty: impl Into<String>, span: Span) -> Self {
        Self {
            name: Ident::new(&name.into(), span),
            ty: Ident::new(&ty.into(), span),
        }
    }
}

impl ToTokens for StorageParam {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let name = &self.name;
        let ty = &self.ty;
        tokens.extend(quote! { #name: #ty });
    }
}

/// Represents different storage types available in smart contracts.
/// This enum handles the differences in storage layout between mappings, arrays, and primitives.
#[derive(Clone, Debug)]
pub enum StorageKind {
    /// Key-value storage like mapping(K => V)
    Mapping(Box<TypeMapping>),
    /// Array storage like T[]
    Array(Box<TypeArray>),
    /// Basic type storage
    Primitive(Box<Type>),
    /// Fixed bytes with original size information
    /// This variant specifically tracks FixedBytes where size > 32,
    /// ensuring proper storage method selection
    FixedBytesExtended(Box<Type>, u16),
}

impl StorageKind {
    /// Parses the arguments needed for this storage type
    pub fn parse_args(&self) -> (Vec<StorageParam>, Option<StorageParam>) {
        match self {
            Self::Mapping(ty) => Self::parse_mapping_args(ty),
            Self::Array(ty) => Self::parse_array_args(ty),
            Self::Primitive(ty) => Self::parse_primitive_args(ty),
            Self::FixedBytesExtended(ty, _) => Self::parse_primitive_args(ty),
        }
    }

    /// Parses arguments for mapping types: mapping(K => V)
    fn parse_mapping_args(input: &TypeMapping) -> (Vec<StorageParam>, Option<StorageParam>) {
        let mut args = Vec::new();
        let mut current_mapping = input;
        let mut i = 0;

        loop {
            let name = if let Some(key_name) = &current_mapping.key_name {
                key_name.0.clone()
            } else {
                Ident::new(&format!("arg{}", i), proc_macro2::Span::call_site())
            };

            let param = StorageParam {
                name,
                ty: Ident::new(
                    &current_mapping.key.to_string(),
                    proc_macro2::Span::call_site(),
                ),
            };

            args.push(param);
            i += 1;

            match &*current_mapping.value {
                Type::Custom(custom_value) => {
                    let output = StorageParam {
                        name: Ident::new("output", proc_macro2::Span::call_site()),
                        ty: Ident::new(&custom_value.to_string(), proc_macro2::Span::call_site()),
                    };

                    return (args, Some(output));
                }
                Type::Mapping(inner_mapping) => {
                    current_mapping = inner_mapping;
                }
                _ => {
                    return (args, None);
                }
            }
        }
    }

    /// Parses arguments for array types: T[]
    fn parse_array_args(ty: &TypeArray) -> (Vec<StorageParam>, Option<StorageParam>) {
        let mut args = Vec::new();
        let mut current_array = ty;
        let mut i = 0;

        loop {
            let param = StorageParam {
                name: Ident::new(&format!("arg{}", i), proc_macro2::Span::call_site()),
                ty: Ident::new("U256", proc_macro2::Span::call_site()),
            };

            args.push(param);
            i += 1;

            match &*current_array.ty {
                Type::Array(inner_array) => {
                    current_array = inner_array;
                }
                Type::Custom(custom_value) => {
                    let output = StorageParam {
                        name: Ident::new("output", proc_macro2::Span::call_site()),
                        ty: Ident::new(&custom_value.to_string(), proc_macro2::Span::call_site()),
                    };
                    return (args, Some(output));
                }
                _ => return (args, None),
            }
        }
    }

    /// Parses arguments for primitive types
    /// Uses the conversion functions to properly convert Solidity types to Rust types
    fn parse_primitive_args(ty: &Type) -> (Vec<StorageParam>, Option<StorageParam>) {
        // Handle special types first
        let type_name = match ty {
            // For FixedBytes, use "FixedBytes" as base name and store size separately
            Type::FixedBytes(_, _) => "FixedBytes".to_string(),

            // For [u8; N] custom type, use "ByteArray" as base name
            Type::Custom(path) => {
                let path_str = path.to_string();
                if path_str.starts_with("[u8; ") && path_str.ends_with("]") {
                    "ByteArray".to_string()
                } else {
                    path_str
                }
            }

            // Standard Solidity types
            Type::Address(_, _) => "Address".to_string(),
            Type::Bool(_) => "bool".to_string(),
            Type::String(_) => "String".to_string(),
            Type::Bytes(_) => "Bytes".to_string(),

            // For other types, use string representation
            _ => ty.to_string(),
        };

        (
            vec![],
            Some(StorageParam {
                name: Ident::new("output", proc_macro2::Span::call_site()),
                ty: Ident::new(&type_name, proc_macro2::Span::call_site()),
            }),
        )
    }

    /// Generates the key calculation logic for this storage type
    pub fn key_calculation(&self, args: &[StorageParam]) -> TokenStream2 {
        match self {
            Self::Mapping(_) => Self::mapping_key_calculation(args),
            Self::Array(_) => Self::array_key_calculation(args),
            Self::Primitive(_) | Self::FixedBytesExtended(_, _) => {
                Self::primitive_key_calculation()
            }
        }
    }

    /// Helper method to get the size of FixedBytes type
    pub fn get_fixed_bytes_size(&self) -> Option<u16> {
        match self {
            Self::FixedBytesExtended(_, original_size) => Some(*original_size),
            Self::Primitive(boxed_ty) => {
                if let Type::FixedBytes(_, size) = &**boxed_ty {
                    Some(size.get())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Helper method to check if this is a FixedBytes type
    pub fn is_fixed_bytes(&self) -> bool {
        match self {
            Self::FixedBytesExtended(_, _) => true,
            Self::Primitive(boxed_ty) => {
                matches!(&**boxed_ty, Type::FixedBytes(_, _))
            }
            _ => false,
        }
    }

    /// Generates key calculation for mapping types
    fn mapping_key_calculation(args: &[StorageParam]) -> TokenStream2 {
        let arguments: Vec<TokenStream2> = args.iter().map(|arg| arg.to_token_stream()).collect();
        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();
        let arg_len = args.len();

        let calculate_keys_fn = quote! {
            fn calculate_keys<SDK: fluentbase_sdk::SharedAPI>(
                sdk: &SDK,
                slot: fluentbase_sdk::U256,
                args: [fluentbase_sdk::U256; #arg_len]
            ) -> fluentbase_sdk::U256 {
                let mut key = slot;
                for arg in args {
                    key = Self::key_hash(sdk, key, arg);
                }
                key
            }
        };

        let key_hash_fn = quote! {
            fn key_hash<SDK: fluentbase_sdk::SharedAPI>(
                sdk: &SDK,
                slot: fluentbase_sdk::U256,
                key: fluentbase_sdk::U256
            ) -> fluentbase_sdk::U256 {
                let mut raw_storage_key: [u8; 64] = [0; 64];

                raw_storage_key[0..32].copy_from_slice(&key.to_be_bytes::<32>());
                raw_storage_key[32..64].copy_from_slice(&slot.to_be_bytes::<32>());

                let storage_key = SDK::keccak256(&raw_storage_key[..]);
                fluentbase_sdk::U256::from_be_bytes(storage_key.0)
            }
        };

        let key_fn = quote! {
            fn key<SDK: fluentbase_sdk::SharedAPI>(sdk: &SDK, #(#arguments),*) -> fluentbase_sdk::U256 {
                let args = [
                    #(
                        fluentbase_sdk::U256::from_be_bytes({
                            let mut buf = fluentbase_sdk::codec::bytes::BytesMut::new();
                            fluentbase_sdk::codec::SolidityABI::encode(&#arg_names, &mut buf, 0).unwrap();
                            let bytes = buf.freeze().to_vec();
                            let mut array = [0u8; 32];
                            let start = 32 - bytes.len();
                            array[start..].copy_from_slice(&bytes);
                            array
                        }),
                    )*
                ];

                Self::calculate_keys(sdk, Self::SLOT, args)
            }
        };

        quote! {
            #calculate_keys_fn
            #key_hash_fn
            #key_fn
        }
    }

    /// Generates key calculation for array types
    fn array_key_calculation(args: &[StorageParam]) -> TokenStream2 {
        let arguments: Vec<TokenStream2> = args.iter().map(|arg| arg.to_token_stream()).collect();
        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        quote! {
            fn key<SDK: fluentbase_sdk::SharedAPI>(sdk: &SDK, #(#arguments),*) -> fluentbase_sdk::U256 {
                use fluentbase_sdk::NativeAPI;
                let mut key = Self::SLOT;

                #(
                    let storage_key = {
                        let storage_key = SDK::keccak256(&key.to_be_bytes::<32>());
                        fluentbase_sdk::U256::from_be_bytes(storage_key.0)
                    };
                    key = storage_key + #arg_names;
                )*

                key
            }
        }
    }

    /// Generates key calculation for primitive types
    fn primitive_key_calculation() -> TokenStream2 {
        quote! {
            fn key<SDK: fluentbase_sdk::SharedAPI>(_sdk: &SDK) -> fluentbase_sdk::U256 {
                Self::SLOT
            }
        }
    }
}

/// Helper function to parse FixedBytes<N> with size tracking
fn parse_fixed_bytes(input: ParseStream, ident: Ident) -> SynResult<StorageKind> {
    // Parse the angle brackets
    let _: Token![<] = input.parse()?;

    // Parse the size
    let size_lit: syn::LitInt = input.parse()?;
    let size = size_lit
        .base10_parse::<u16>()
        .map_err(|_| Error::new(size_lit.span(), "Expected a numeric size for FixedBytes"))?;

    // Consume the '>'
    let _: Token![>] = input.parse()?;

    // Store the original size for proper type generation
    let original_size = size;

    // For internal representation, we cap at 32 bytes
    // But we store the original size if it's larger
    let capped_size = size.min(32);

    if let Some(nonzero_size) = NonZeroU16::new(capped_size) {
        let span = ident.span();

        // If size exceeds 32, use the extended variant
        if size > 32 {
            let ty = Box::new(Type::FixedBytes(span, nonzero_size));
            return Ok(StorageKind::FixedBytesExtended(ty, original_size));
        } else {
            // For sizes <= 32, use the standard primitive type
            let ty = Box::new(Type::FixedBytes(span, nonzero_size));
            return Ok(StorageKind::Primitive(ty));
        }
    }

    // Fallback for invalid size
    Err(Error::new(ident.span(), "Invalid size for FixedBytes"))
}

impl Parse for StorageKind {
    fn parse(input: ParseStream) -> SynResult<Self> {
        // Check if this is FixedBytes<N>
        if input.peek(Ident::peek_any) {
            let ident_fork = input.fork();
            let ident: Ident = ident_fork.parse()?;

            if ident == "FixedBytes" {
                // Consume the FixedBytes identifier
                let _: Ident = input.parse()?;

                // Parse the angle brackets
                if input.peek(Token![<]) {
                    return parse_fixed_bytes(input, ident);
                }
            }
        }

        // Check if this is [u8; N]
        if input.peek(syn::token::Bracket) {
            let content;
            syn::bracketed!(content in input);

            // Try to parse u8
            if content.peek(Ident::peek_any) {
                let elem_type: Ident = content.parse()?;

                if elem_type == "u8" && content.peek(Token![;]) {
                    // Parse the semicolon and size
                    let _: Token![;] = content.parse()?;
                    let size_lit: syn::LitInt = content.parse()?;
                    let size = size_lit.base10_parse::<u16>().map_err(|_| {
                        Error::new(size_lit.span(), "Expected a numeric size for [u8; N]")
                    })?;

                    // Store original size for proper handling
                    let original_size = size;

                    // For internal representation, cap at 32 bytes
                    let capped_size = size.min(32);

                    if let Some(nonzero_size) = NonZeroU16::new(capped_size) {
                        let span = elem_type.span();

                        // If size exceeds 32, use the extended variant
                        if size > 32 {
                            let ty = Box::new(Type::FixedBytes(span, nonzero_size));
                            return Ok(StorageKind::FixedBytesExtended(ty, original_size));
                        } else {
                            // For sizes <= 32, use the standard primitive type
                            let ty = Box::new(Type::FixedBytes(span, nonzero_size));
                            return Ok(StorageKind::Primitive(ty));
                        }
                    }
                }
            }
        }

        // Try standard Solidity type parsing
        let fork = input.fork();
        if let Ok(ty) = fork.parse::<TypeMapping>() {
            input.advance_to(&fork);
            return Ok(Self::Mapping(Box::new(ty)));
        }

        let fork = input.fork();
        if let Ok(ty) = fork.parse::<Type>() {
            input.advance_to(&fork);
            match ty {
                Type::Array(array) => return Ok(Self::Array(Box::new(array))),
                _ => return Ok(Self::Primitive(Box::new(ty))),
            }
        }

        Err(input.error("Failed to parse storage type"))
    }
}

/// Represents a single storage slot in the contract with its associated metadata.
/// Each slot can contain different types (primitive, mapping, array) and has methods
/// for generating appropriate getters and setters.
#[derive(Clone, Debug)]
pub struct StorageSlot {
    /// The numerical index of the storage slot
    slot: usize,
    /// The name of the storage variable
    name: Ident,
    /// The type of storage (mapping, array, or primitive)
    kind: StorageKind,
    /// Function arguments for accessing this storage
    args: Vec<StorageParam>,
    /// Return type for getter methods
    output: Option<StorageParam>,
}

impl StorageSlot {
    /// Determines if a type can use direct storage access.
    ///
    /// Direct storage is possible for primitive Solidity types that are smaller than 32 bytes
    /// and don't require additional encoding/decoding.
    fn can_use_direct_storage(&self) -> bool {
        let Some(output) = &self.output else {
            return false;
        };

        let type_name = output.ty.to_string();

        // Check for Address type (20 bytes)
        if type_name == "Address" {
            return true;
        }

        // Check for boolean type
        if type_name == "bool" {
            return true;
        }

        // Check for unsigned integers
        match type_name.as_str() {
            "u8" | "u16" | "u32" | "u64" | "u128" | "U256" => return true,
            _ => {}
        }

        // Check for signed integers
        match type_name.as_str() {
            "i8" | "i16" | "i32" | "i64" | "i128" | "I256" => return true,
            _ => {}
        }

        if type_name == "FixedBytes" || type_name == "ByteArray" {
            // Check if it's a FixedBytes type and use its size
            if self.kind.is_fixed_bytes() {
                if let Some(size) = self.kind.get_fixed_bytes_size() {
                    return size <= 32;
                }
            }
        }

        false
    }

    /// Helper method to get the appropriate output type token stream
    fn get_output_type(&self) -> TokenStream2 {
        match &self.output {
            Some(output) => {
                if output.ty == "FixedBytes" || output.ty == "ByteArray" {
                    if self.kind.is_fixed_bytes() {
                        if let Some(size) = self.kind.get_fixed_bytes_size() {
                            let size_value = size as usize;
                            return quote! { FixedBytes<#size_value> };
                        }
                    }
                    output.ty.to_token_stream()
                } else {
                    output.ty.to_token_stream()
                }
            }
            None => quote! { () },
        }
    }

    /// Generates the slot definition code
    fn slot_definition(&self) -> TokenStream2 {
        let slot = self.slot as u64;
        quote! {
            const SLOT: fluentbase_sdk::U256 = fluentbase_sdk::U256::from_limbs([#slot, 0u64, 0u64, 0u64]);
        }
    }

    /// Generates the getter method code
    fn getter_definition(&self) -> TokenStream2 {
        let arguments: Vec<TokenStream2> =
            self.args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = self.args.iter().map(|arg| &arg.name).collect();

        let output = self.get_output_type();

        let get_args = if arguments.is_empty() {
            quote! { sdk: &SDK }
        } else {
            quote! { sdk: &SDK, #(#arguments),* }
        };

        if self.can_use_direct_storage() {
            if arguments.is_empty() {
                quote! {
                    fn get<SDK: fluentbase_sdk::SharedAPI>(#get_args) -> #output {
                        let key = Self::key(sdk);
                        <#output as fluentbase_sdk::storage_legacy::DirectStorage<SDK>>::get(sdk, key)
                    }
                }
            } else {
                quote! {
                    fn get<SDK: fluentbase_sdk::SharedAPI>(#get_args) -> #output {
                        let key = Self::key(sdk, #(#arg_names),*);
                        <#output as fluentbase_sdk::storage_legacy::DirectStorage<SDK>>::get(sdk, key)
                    }
                }
            }
        } else if arguments.is_empty() {
            quote! {
                fn get<SDK: fluentbase_sdk::SharedAPI>(#get_args) -> #output {
                    let key = Self::key(sdk);
                    <#output as fluentbase_sdk::storage_legacy::StorageValueSolidity<SDK, #output>>::get(sdk, key)
                }
            }
        } else {
            quote! {
                fn get<SDK: fluentbase_sdk::SharedAPI>(#get_args) -> #output {
                    let key = Self::key(sdk, #(#arg_names),*);
                    <#output as fluentbase_sdk::storage_legacy::StorageValueSolidity<SDK, #output>>::get(sdk, key)
                }
            }
        }
    }

    /// Generates the setter method code
    fn setter_definition(&self) -> TokenStream2 {
        let arguments: Vec<TokenStream2> =
            self.args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = self.args.iter().map(|arg| &arg.name).collect();

        let output = self.get_output_type();

        let set_args = if arguments.is_empty() {
            quote! { sdk: &mut SDK, value: #output }
        } else {
            quote! { sdk: &mut SDK, #(#arguments),*, value: #output }
        };

        if self.can_use_direct_storage() {
            if arguments.is_empty() {
                quote! {
                    fn set<SDK: fluentbase_sdk::SharedAPI>(#set_args) {
                        let key = Self::key(sdk);
                        <#output as fluentbase_sdk::storage_legacy::DirectStorage<SDK>>::set(sdk, key, value)
                    }
                }
            } else {
                quote! {
                    fn set<SDK: fluentbase_sdk::SharedAPI>(#set_args) {
                        let key = Self::key(sdk, #(#arg_names),*);
                        <#output as fluentbase_sdk::storage_legacy::DirectStorage<SDK>>::set(sdk, key, value)
                    }
                }
            }
        } else {
            // Fallback to StorageValueSolidity for complex types
            if arguments.is_empty() {
                quote! {
                    fn set<SDK: fluentbase_sdk::SharedAPI>(#set_args) {
                        use fluentbase_sdk::storage_legacy::StorageValueSolidity;
                        let key = Self::key(sdk);
                        <#output as fluentbase_sdk::storage_legacy::StorageValueSolidity<SDK, #output>>::set(sdk, key, value.clone());
                    }
                }
            } else {
                quote! {
                    fn set<SDK: fluentbase_sdk::SharedAPI>(#set_args) {
                        use fluentbase_sdk::storage_legacy::StorageValueSolidity;
                        let key = Self::key(sdk, #(#arg_names),*);
                        <#output as fluentbase_sdk::storage_legacy::StorageValueSolidity<SDK, #output>>::set(sdk, key, value.clone());
                    }
                }
            }
        }
    }

    /// Generates the complete struct and implementation for this storage slot
    fn generate(&self) -> TokenStream2 {
        let name = &self.name;
        let slot = self.slot_definition();
        let getter = self.getter_definition();
        let setter = self.setter_definition();
        let key_calc = self.kind.key_calculation(&self.args);

        quote! {
            pub struct #name {}
            impl #name {
                #slot
                #getter
                #setter
                #key_calc
            }
        }
    }
}

impl Parse for StorageSlot {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let kind: StorageKind = match input.parse() {
            Ok(kind) => kind,
            Err(err) => abort!(input.span(), "Failed to parse storage type: {}", err;
                help = "Expected a valid Solidity type like 'Address', 'U256', 'mapping(K => V)', or 'T[]'"),
        };

        let name: Ident = match input.parse() {
            Ok(name) => name,
            Err(_) => abort!(input.span(), "Expected an identifier for storage slot name"),
        };

        let (args, output) = kind.parse_args();

        let slot = Self {
            slot: 0,
            name,
            kind,
            args,
            output,
        };

        Ok(slot)
    }
}

impl ToTokens for StorageSlot {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        tokens.extend(self.generate());
    }
}

/// Collection of storage slots for a contract.
/// This represents all storage variables defined in a solidity_storage! macro.
#[derive(Default)]
pub struct Storage {
    /// List of storage slots in the contract
    slots: Vec<StorageSlot>,
}

impl Storage {
    /// Creates a new empty storage collection
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a storage slot to the collection, assigning it the next available slot number
    pub fn add_slot(&mut self, mut slot: StorageSlot) {
        slot.slot = self.slots.len();
        self.slots.push(slot);
    }

    /// Generates the complete code for all storage slots
    pub fn generate(&self) -> TokenStream2 {
        let slots = &self.slots;

        quote! {
            #(#slots)*
        }
    }
}

impl Parse for Storage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let slots: Punctuated<StorageSlot, Semi> =
            match input.parse_terminated(StorageSlot::parse, Semi) {
                Ok(slots) => slots,
                Err(err) => abort_call_site!("Failed to parse storage definition: {}", err;
                help = "Check that your storage definition follows the correct syntax"),
            };

        let mut storage = Self::new();

        for slot in slots {
            storage.add_slot(slot);
        }

        if storage.slots.is_empty() {
            abort_call_site!("Storage definition contains no slots";
                help = "Add at least one storage variable like 'Address Owner;'")
        }

        Ok(storage)
    }
}

impl ToTokens for Storage {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        tokens.extend(self.generate());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use syn::parse_quote;

    #[test]
    fn test_primitive_storage() {
        let storage: Storage = parse_quote! {
            Address Owner;
            bool Paused;
            U256 TotalSupply;
        };

        let generated = storage.to_token_stream();

        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("primitive_storage", formatted);
    }

    #[test]
    fn test_mapping_storage() {
        let storage: Storage = parse_quote! {
            mapping(Address => U256) Balance;
            mapping(Address => Bytes) ArbitraryData;
            mapping(Address => mapping(Address => U256)) Allowance;
        };

        let generated = storage.to_token_stream();

        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("mapping_storage", formatted);
    }

    #[test]
    fn test_array_storage() {
        let storage: Storage = parse_quote! {
            U256[] Arr;
            Address[][][] NestedArr;
        };

        let generated = storage.to_token_stream();

        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("array_storage", formatted);
    }

    #[test]
    fn test_fixed_bytes_storage() {
        let storage: Storage = parse_quote! {
            FixedBytes<32> CustomBytes1;
            [u8; 32] CustomBytes2;
            FixedBytes<321> CustomBytes1;
            [u8; 321] CustomBytes2;

        };

        let generated = storage.to_token_stream();

        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("fixed_bytes_storage", formatted);
    }
}
