use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{
    parse::{discouraged::Speculative, Parse, ParseStream},
    punctuated::Punctuated,
    token::Semi,
    Result as SynResult,
};
use syn_solidity::{Type, TypeArray, TypeMapping};

/// Error types for storage operations
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Parse error: {0}")]
    Parse(#[from] syn::Error),
    #[error("Invalid storage type: {0}")]
    InvalidType(String),
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Storage parameter with type information
#[derive(Clone, Debug)]
pub struct StorageParam {
    name: Ident,
    ty: Ident,
}

impl StorageParam {
    pub fn new(name: impl Into<String>, ty: impl Into<String>) -> Self {
        Self::with_span(name, ty, Span::call_site())
    }

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

/// Represents different storage types available in smart contracts
#[derive(Clone, Debug)]
pub enum StorageKind {
    /// Key-value storage like mapping(K => V)
    Mapping(Box<TypeMapping>),
    /// Array storage like T[]
    Array(Box<TypeArray>),
    /// Basic type storage
    Primitive(Box<Type>),
}

impl StorageKind {
    pub fn parse_args(&self) -> (Vec<StorageParam>, Option<StorageParam>) {
        match self {
            Self::Mapping(ty) => Self::parse_mapping_args(ty),
            Self::Array(ty) => Self::parse_array_args(ty),
            Self::Primitive(ty) => Self::parse_primitive_args(ty),
        }
    }

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

    fn parse_primitive_args(ty: &Type) -> (Vec<StorageParam>, Option<StorageParam>) {
        (
            vec![],
            Some(StorageParam {
                name: Ident::new("output", proc_macro2::Span::call_site()),
                ty: Ident::new(&ty.to_string(), proc_macro2::Span::call_site()),
            }),
        )
    }

    pub fn key_calculation(&self, args: &[StorageParam]) -> TokenStream2 {
        match self {
            Self::Mapping(ty) => Self::mapping_key_calculation(ty, args),
            Self::Array(ty) => Self::array_key_calculation(ty, args),
            Self::Primitive(_) => Self::primitive_key_calculation(),
        }
    }

    fn mapping_key_calculation(_ty: &TypeMapping, args: &[StorageParam]) -> TokenStream2 {
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
                raw_storage_key[0..32].copy_from_slice(slot.as_le_slice());
                raw_storage_key[32..64].copy_from_slice(key.as_le_slice());
                use fluentbase_sdk::NativeAPI;
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

    fn array_key_calculation(ty: &TypeArray, args: &[StorageParam]) -> TokenStream2 {
        let arguments: Vec<TokenStream2> = args.iter().map(|arg| arg.to_token_stream()).collect();
        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        quote! {
            fn key<SDK: fluentbase_sdk::SharedAPI>(sdk: &SDK, #(#arguments),*) -> fluentbase_sdk::U256 {
                use fluentbase_sdk::NativeAPI;
                let mut key = Self::SLOT;

                #(
                    let storage_key = {
                        let storage_key = SDK::keccak256(key.as_le_slice());
                        U256::from_be_bytes(storage_key.0)
                    };
                    key = storage_key + #arg_names;
                )*

                key
            }
        }
    }

    fn primitive_key_calculation() -> TokenStream2 {
        quote! {
            fn key<SDK: fluentbase_sdk::SharedAPI>(_sdk: &SDK) -> fluentbase_sdk::U256 {
                Self::SLOT
            }
        }
    }
}

impl Parse for StorageKind {
    fn parse(input: ParseStream) -> SynResult<Self> {
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

        if let Ok(ty) = input.parse::<Type>() {
            return Ok(Self::Primitive(Box::new(ty)));
        }

        Err(input.error("Failed to parse storage type"))
    }
}

/// Represents a single storage slot in the contract
#[derive(Clone, Debug)]
pub struct StorageSlot {
    slot: usize,
    name: Ident,
    kind: StorageKind,
    args: Vec<StorageParam>,
    output: Option<StorageParam>,
}

impl StorageSlot {
    fn slot_definition(&self) -> TokenStream2 {
        let slot = self.slot as u64;
        quote! {
            const SLOT: fluentbase_sdk::U256 = fluentbase_sdk::U256::from_limbs([#slot, 0u64, 0u64, 0u64]);
        }
    }

    // fn format_args(&self) -> (TokenStream2, Vec<&Ident>) {
    //     let args_tokens: Vec<_> = self.args.iter().map(ToTokens::to_token_stream).collect();
    //     let arg_names: Vec<_> = self.args.iter().map(|arg| &arg.name).collect();

    //     let method_args = if self.args.is_empty() {
    //         quote! { sdk: &SDK }
    //     } else {
    //         quote! { sdk: &SDK, #(#args_tokens),* }
    //     };

    //     (method_args, arg_names)
    // }

    fn getter_definition(&self) -> TokenStream2 {
        let arguments: Vec<TokenStream2> =
            self.args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = self.args.iter().map(|arg| &arg.name).collect();

        let output = match &self.output {
            Some(output) => output.ty.to_token_stream(),
            None => quote! { () },
        };

        let get_args = if arguments.is_empty() {
            quote! { sdk: &SDK }
        } else {
            quote! { sdk: &SDK, #(#arguments),* }
        };

        quote! {
            fn get<SDK: fluentbase_sdk::SharedAPI>(#get_args) -> #output {
                let key = Self::key(sdk, #(#arg_names),*);
                <#output as fluentbase_sdk::storage::StorageValueFluent<SDK, #output>>::get(sdk, key)
            }
        }
    }

    fn setter_definition(&self) -> TokenStream2 {
        let arguments: Vec<TokenStream2> =
            self.args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = self.args.iter().map(|arg| &arg.name).collect();

        let output = match &self.output {
            Some(output) => output.ty.to_token_stream(),
            None => quote! { () },
        };

        let set_args = if arguments.is_empty() {
            quote! { sdk: &mut SDK, value: #output }
        } else {
            quote! { sdk: &mut SDK, #(#arguments),*, value: #output }
        };

        quote! {
            fn set<SDK: fluentbase_sdk::SharedAPI>(#set_args) {
                use fluentbase_sdk::storage::StorageValueFluent;
                let key = Self::key(sdk, #(#arg_names),*);
                <#output as fluentbase_sdk::storage::StorageValueFluent<SDK, #output>>::set(sdk, key, value.clone());
            }
        }
    }
}

impl Parse for StorageSlot {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let kind: StorageKind = input.parse()?;
        let name: Ident = input.parse()?;
        let (args, output) = kind.parse_args();

        Ok(Self {
            slot: 0,
            name,
            kind,
            args,
            output,
        })
    }
}

impl ToTokens for StorageSlot {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let name = &self.name;
        let slot = self.slot_definition();
        let getter = self.getter_definition();
        let setter = self.setter_definition();
        let key_calc = self.kind.key_calculation(&self.args);

        tokens.extend(quote! {
            pub struct #name {}
            impl #name {
                #slot
                #getter
                #setter
                #key_calc
            }
        });
    }
}

/// Collection of storage slots
#[derive(Default)]
pub struct Storage {
    slots: Vec<StorageSlot>,
}

impl Storage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_slot(&mut self, mut slot: StorageSlot) {
        slot.slot = self.slots.len();
        self.slots.push(slot);
    }
}

impl Parse for Storage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let slots: Punctuated<StorageSlot, Semi> =
            input.parse_terminated(StorageSlot::parse, Semi)?;
        let mut storage = Self::new();

        for slot in slots {
            storage.add_slot(slot);
        }

        Ok(storage)
    }
}

impl ToTokens for Storage {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let slots = &self.slots;
        tokens.extend(quote! {
            #(#slots)*
        });
    }
}
